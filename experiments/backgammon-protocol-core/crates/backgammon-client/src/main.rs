#[cfg(target_arch = "wasm32")]
mod components;

pub mod commitment_planner;
pub mod controller;
pub mod ledger_codec;
pub mod local_role_store;
pub mod pending_action;
pub mod pending_action_store;
pub mod projection;
pub mod reveal_planner;
pub mod secret_store;
pub mod transport;

#[cfg(target_arch = "wasm32")]
mod browser {
    use backgammon_core::{Dice, GameStatus, MoveSource, MoveTarget, Player, TurnPhase};
    use backgammon_protocol::{replay_game, DiceSecret, GameActionPayload};
    use yew::prelude::*;

    use crate::commitment_planner::{plan_commitment, CommitmentPlan, CommitmentPlannerInput};
    use crate::components::board::Board;
    use crate::components::controls::GameControls;
    use crate::components::dice::DiceDisplay;
    use crate::components::history::MoveHistory;
    use crate::components::player_panel::PlayerPanel;
    use crate::controller::{LocalGameController, LocalGameOutcome};
    use crate::ledger_codec::decode_verified_ledger;
    use crate::local_role_store::{load_local_role, store_local_role};
    use crate::pending_action_store::{
        load_pending_action, remove_pending_action, store_pending_action,
    };
    use crate::projection::BoardView;
    use crate::secret_store::{load_dice_secret, store_dice_secret};
    use crate::transport::{
        classify_response, connect, request_test_contract, submit_action_delta,
        submit_first_create_delta, ClassifiedResponse, ConnectionStatus, ContractProbeStatus,
        SubscriptionStatus, TEST_CONTRACT_ID,
    };

    fn secure_local_dice() -> Result<Dice, String> {
        let window =
            web_sys::window().ok_or_else(|| "Browser window is unavailable.".to_owned())?;

        let crypto = window
            .crypto()
            .map_err(|error| format!("Browser randomness is unavailable: {error:?}"))?;

        let mut dice = [0_u8; 2];
        let mut accepted = 0_usize;

        while accepted < dice.len() {
            let mut random_bytes = [0_u8; 8];

            crypto
                .get_random_values_with_u8_array(&mut random_bytes)
                .map_err(|error| format!("Could not generate dice: {error:?}"))?;

            for byte in random_bytes {
                if byte < 252 {
                    dice[accepted] = byte % 6 + 1;
                    accepted += 1;

                    if accepted == dice.len() {
                        break;
                    }
                }
            }
        }

        Ok(Dice {
            first: dice[0],
            second: dice[1],
        })
    }

    fn secure_random_32(purpose: &str) -> Result<[u8; 32], String> {
        let window =
            web_sys::window().ok_or_else(|| "Browser window is unavailable.".to_owned())?;

        let crypto = window
            .crypto()
            .map_err(|error| format!("Browser randomness is unavailable: {error:?}"))?;

        let mut bytes = [0_u8; 32];

        crypto
            .get_random_values_with_u8_array(&mut bytes)
            .map_err(|error| format!("Could not generate {purpose}: {error:?}"))?;

        Ok(bytes)
    }

    fn local_commitment_storage_context(
        authoritative_state: &[u8],
        local_player: Player,
    ) -> Result<Option<([u8; 32], u32, Player)>, String> {
        let ledger = decode_verified_ledger(authoritative_state)?;

        let replay = replay_game(ledger.typed_actions())
            .map_err(|error| format!("Could not replay verified browser state: {error:?}"))?;

        Ok(ledger.typed_actions().iter().find_map(|record| {
            let GameActionPayload::CommitDice { turn, player, .. } = &record.payload else {
                return None;
            };

            (*turn == replay.next_turn && *player == local_player).then_some((
                record.game_id,
                *turn,
                *player,
            ))
        }))
    }

    fn plan_browser_commitment(
        authoritative_state: &[u8],
        local_player: Player,
    ) -> Result<CommitmentPlan, String> {
        let pending = load_pending_action(TEST_CONTRACT_ID)?;

        let stored_secret = if let Some(pending) = pending.as_ref() {
            let record = pending.verify()?;

            let GameActionPayload::CommitDice { turn, player, .. } = &record.payload else {
                return Err("Stored pending action is not a dice commitment.".to_owned());
            };

            if *player != local_player {
                return Err(
                    "Stored pending commitment belongs to a different local player.".to_owned(),
                );
            }

            load_dice_secret(TEST_CONTRACT_ID, &pending.game_id, *turn, *player)?
        } else if let Some((game_id, turn, player)) =
            local_commitment_storage_context(authoritative_state, local_player)?
        {
            load_dice_secret(TEST_CONTRACT_ID, &game_id, turn, player)?
        } else {
            None
        };

        /*
         * Entropy is supplied only as candidate material. The planner decides
         * whether the verified authoritative state permits a new commitment.
         * Candidate material is never stored unless the planner returns a
         * newly created Submit plan.
         */
        let new_secret = if pending.is_none() && stored_secret.is_none() {
            Some(secure_random_32("local dice secret")?)
        } else {
            None
        };

        let new_action_id = if pending.is_none() && stored_secret.is_none() {
            Some(secure_random_32("network action ID")?)
        } else {
            None
        };

        let plan = plan_commitment(CommitmentPlannerInput {
            contract_id: TEST_CONTRACT_ID,
            local_player,
            authoritative_state,
            pending: pending.as_ref(),
            stored_secret,
            new_secret,
            new_action_id,
        })?;

        match &plan {
            CommitmentPlan::NoAction => {}

            CommitmentPlan::Accepted { .. } => {
                if pending.is_some() {
                    remove_pending_action(TEST_CONTRACT_ID)?;
                }
            }

            CommitmentPlan::Submit {
                secret,
                pending,
                recovered_pending,
            } => {
                if !recovered_pending {
                    let record = pending.verify()?;

                    let GameActionPayload::CommitDice { turn, player, .. } = record.payload else {
                        return Err(
                            "New commitment plan produced a non-commitment action.".to_owned()
                        );
                    };

                    if player != local_player {
                        return Err(
                            "New commitment plan produced an action for another player.".to_owned()
                        );
                    }

                    /*
                     * Persist the secret before the action that commits to it.
                     * A crash must never leave a retryable commitment without
                     * its corresponding reveal material.
                     */
                    store_dice_secret(TEST_CONTRACT_ID, &pending.game_id, turn, player, secret)?;

                    /*
                     * Persist the exact encoded delta before network
                     * submission. Retries must use these same bytes.
                     */
                    store_pending_action(pending)?;
                }
            }
        }

        Ok(plan)
    }

    fn choose_local_role(player: Player) -> Result<(), String> {
        if load_pending_action(TEST_CONTRACT_ID)?.is_some() {
            return Err("The local role cannot change while an action is pending.".to_owned());
        }

        match load_local_role(TEST_CONTRACT_ID)? {
            Some(existing) if existing == player => Ok(()),

            Some(existing) => Err(format!(
                "This browser profile is already locked as {} for this contract.",
                player_name(existing),
            )),

            None => store_local_role(TEST_CONTRACT_ID, player),
        }
    }

    fn player_name(player: Player) -> &'static str {
        match player {
            Player::White => "White",
            Player::Black => "Black",
        }
    }

    fn result_name(points: u8) -> &'static str {
        match points {
            1 => "Single game",
            2 => "Gammon",
            3 => "Backgammon",
            _ => "Game",
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PendingConfirmation {
        Resign,
        Leave,
        NewGame,
    }

    impl PendingConfirmation {
        fn title(self) -> &'static str {
            match self {
                Self::Resign => "Resign this game?",
                Self::Leave => "Leave the table?",
                Self::NewGame => "Start a new game?",
            }
        }

        fn message(self) -> &'static str {
            match self {
                Self::Resign => "The active player will resign and the opponent will win.",
                Self::Leave => "The current local session will end.",
                Self::NewGame => "The current board, dice, and move history will be discarded.",
            }
        }

        fn confirm_label(self) -> &'static str {
            match self {
                Self::Resign => "Resign game",
                Self::Leave => "Leave table",
                Self::NewGame => "Start new game",
            }
        }
    }

    #[function_component(App)]
    fn app() -> Html {
        let controller = use_state(LocalGameController::new);
        let interface_error = use_state(|| None::<String>);
        let pending_confirmation = use_state(|| None::<PendingConfirmation>);
        let connection_status = use_state(|| ConnectionStatus::Disconnected);
        let contract_status = use_state(|| ContractProbeStatus::WaitingForConnection);
        let subscription_status = use_state(|| SubscriptionStatus::Pending);
        let freenet_api = use_mut_ref(|| None::<freenet_stdlib::client_api::WebApi>);
        let first_delta_submitted = use_mut_ref(|| false);
        let local_commitment_submitted = use_mut_ref(|| false);
        let local_dice_secret = use_mut_ref(|| None::<DiceSecret>);
        let dice_secret_status = use_state(|| "Checking browser storage".to_owned());

        /*
         * The stored role is scoped to this exact contract instance.
         * An invalid stored value is retained as an error rather than silently
         * converted into a player role.
         */
        let local_role = use_state(|| load_local_role(TEST_CONTRACT_ID));

        let selected_local_role = match &*local_role {
            Ok(role) => *role,
            Err(_) => None,
        };

        {
            let connection_status = connection_status.clone();
            let contract_status = contract_status.clone();
            let subscription_status = subscription_status.clone();
            let freenet_api = freenet_api.clone();
            let first_delta_submitted = first_delta_submitted.clone();
            let local_commitment_submitted = local_commitment_submitted.clone();
            let local_dice_secret = local_dice_secret.clone();
            let dice_secret_status = dice_secret_status.clone();

            use_effect_with(selected_local_role, move |selected_role| {
                let selected_role = *selected_role;

                /*
                 * Replacing the role dependency replaces the active transport
                 * closure. Any durable pending action remains in storage.
                 */
                freenet_api.borrow_mut().take();
                *local_commitment_submitted.borrow_mut() = false;
                *local_dice_secret.borrow_mut() = None;

                let status_for_callback = connection_status.clone();
                let contract_for_response = contract_status.clone();
                let subscription_for_response = subscription_status.clone();
                let subscription_for_status = subscription_status.clone();
                let api_for_response = freenet_api.clone();
                let submitted_for_response = first_delta_submitted.clone();
                let commitment_for_response = local_commitment_submitted.clone();
                let secret_for_response = local_dice_secret.clone();
                let secret_status_for_response = dice_secret_status.clone();

                let api_for_open = freenet_api.clone();
                let connection_for_open = connection_status.clone();
                let contract_for_open = contract_status.clone();
                let subscription_for_open = subscription_status.clone();

                match connect(
                    move |status| {
                        match &status {
                            ConnectionStatus::Connecting => {
                                subscription_for_status.set(SubscriptionStatus::Pending);
                            }
                            ConnectionStatus::Connected => {}
                            ConnectionStatus::Disconnected | ConnectionStatus::Failed(_) => {
                                subscription_for_status.set(SubscriptionStatus::Inactive);
                            }
                        }

                        status_for_callback.set(status);
                    },
                    move |response| {
                        if let Some(classified) = classify_response(response) {
                            let ClassifiedResponse {
                                contract_status,
                                subscription_status,
                                contract_key,
                                authoritative_state,
                                should_submit_first_delta,
                            } = classified;

                            if let Some(contract) = contract_status {
                                contract_for_response.set(contract);
                            }

                            if let Some(subscription) = subscription_status {
                                subscription_for_response.set(subscription);
                            }

                            if should_submit_first_delta {
                                let Some(key) = contract_key else {
                                    contract_for_response.set(ContractProbeStatus::Failed(
                                        "The empty ledger response did not include a full contract key."
                                            .to_owned(),
                                    ));
                                    return;
                                };

                                {
                                    let mut submitted = submitted_for_response.borrow_mut();

                                    if *submitted {
                                        return;
                                    }

                                    *submitted = true;
                                }

                                contract_for_response.set(ContractProbeStatus::Updating);

                                let api_for_update = api_for_response.clone();
                                let contract_for_update = contract_for_response.clone();
                                let submitted_for_update = submitted_for_response.clone();

                                wasm_bindgen_futures::spawn_local(async move {
                                    let submit_result = {
                                        let mut api = api_for_update.borrow_mut();

                                        match api.as_mut() {
                                            Some(api) => submit_first_create_delta(api, key).await,
                                            None => {
                                                Err("Freenet connection closed before the update."
                                                    .to_owned())
                                            }
                                        }
                                    };

                                    match submit_result {
                                        Ok(()) => {
                                            contract_for_update
                                                .set(ContractProbeStatus::VerifyingUpdate);

                                            gloo_timers::future::TimeoutFuture::new(750).await;

                                            let refresh_result = {
                                                let mut api = api_for_update.borrow_mut();

                                                match api.as_mut() {
                                                    Some(api) => request_test_contract(api).await,
                                                    None => Err(
                                                        "Freenet connection closed before update verification."
                                                            .to_owned(),
                                                    ),
                                                }
                                            };

                                            if let Err(error) = refresh_result {
                                                contract_for_update
                                                    .set(ContractProbeStatus::Failed(error));
                                            }
                                        }
                                        Err(error) => {
                                            *submitted_for_update.borrow_mut() = false;
                                            contract_for_update
                                                .set(ContractProbeStatus::Failed(error));
                                        }
                                    }
                                });
                            }
                            if let (Some(key), Some(state_bytes)) =
                                (contract_key, authoritative_state)
                            {
                                let Some(local_player) = selected_role else {
                                    secret_status_for_response
                                        .set("Select White or Black to join the game".to_owned());
                                    return;
                                };

                                let plan = match plan_browser_commitment(&state_bytes, local_player)
                                {
                                    Ok(plan) => plan,
                                    Err(error) => {
                                        secret_status_for_response
                                            .set(format!("Recovery failed: {error}"));

                                        contract_for_response
                                            .set(ContractProbeStatus::Failed(error));

                                        return;
                                    }
                                };

                                match plan {
                                    CommitmentPlan::NoAction => {}

                                    CommitmentPlan::Accepted { secret } => {
                                        *secret_for_response.borrow_mut() = Some(secret);
                                        *commitment_for_response.borrow_mut() = true;

                                        secret_status_for_response.set(
                                            "Recovered and matched accepted commitment".to_owned(),
                                        );
                                    }

                                    CommitmentPlan::Submit {
                                        secret,
                                        pending,
                                        recovered_pending,
                                    } => {
                                        let delta = pending.delta;
                                        {
                                            let mut submitted =
                                                commitment_for_response.borrow_mut();

                                            if *submitted {
                                                return;
                                            }

                                            *submitted = true;
                                        }

                                        *secret_for_response.borrow_mut() = Some(secret);

                                        secret_status_for_response.set(if recovered_pending {
                                            "Recovered exact pending action; retrying".to_owned()
                                        } else {
                                            "Stored locally; awaiting network verification"
                                                .to_owned()
                                        });

                                        contract_for_response.set(ContractProbeStatus::Updating);

                                        let api_for_update = api_for_response.clone();
                                        let contract_for_update = contract_for_response.clone();

                                        wasm_bindgen_futures::spawn_local(async move {
                                            let submit_result = {
                                                let mut api = api_for_update.borrow_mut();

                                                match api.as_mut() {
                                                    Some(api) => {
                                                        submit_action_delta(
                                                            api,
                                                            key,
                                                            delta,
                                                        )
                                                        .await
                                                    }
                                                    None => Err(
                                                        "Freenet connection closed before the pending commitment update."
                                                            .to_owned(),
                                                    ),
                                                }
                                            };

                                            match submit_result {
                                                Ok(()) => {
                                                    contract_for_update
                                                        .set(ContractProbeStatus::VerifyingUpdate);

                                                    gloo_timers::future::TimeoutFuture::new(750)
                                                        .await;

                                                    let refresh_result = {
                                                        let mut api = api_for_update.borrow_mut();

                                                        match api.as_mut() {
                                                            Some(api) => {
                                                                request_test_contract(api)
                                                                    .await
                                                            }
                                                            None => Err(
                                                                "Freenet connection closed before commitment verification."
                                                                    .to_owned(),
                                                            ),
                                                        }
                                                    };

                                                    if let Err(error) = refresh_result {
                                                        contract_for_update.set(
                                                            ContractProbeStatus::Failed(error),
                                                        );
                                                    }
                                                }

                                                Err(error) => {
                                                    /*
                                                     * Do not clear the durable pending record
                                                     * and do not regenerate the action. A later
                                                     * reconnect will reconcile and retry these
                                                     * exact stored bytes.
                                                     */
                                                    contract_for_update
                                                        .set(ContractProbeStatus::Failed(error));
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    },
                    move || {
                        let api_for_request = api_for_open.clone();
                        let connection_for_request = connection_for_open.clone();
                        let contract_for_request = contract_for_open.clone();
                        let subscription_for_request = subscription_for_open.clone();

                        wasm_bindgen_futures::spawn_local(async move {
                            contract_for_request.set(ContractProbeStatus::Requesting);

                            subscription_for_request.set(SubscriptionStatus::Pending);

                            let result = {
                                let mut api = api_for_request.borrow_mut();

                                match api.as_mut() {
                                    Some(api) => request_test_contract(api).await,
                                    None => Err(
                                        "Freenet WebSocket opened without an active API handle."
                                            .to_owned(),
                                    ),
                                }
                            };

                            if let Err(error) = result {
                                connection_for_request.set(ConnectionStatus::Failed(error.clone()));

                                contract_for_request.set(ContractProbeStatus::Failed(error));

                                subscription_for_request.set(SubscriptionStatus::Inactive);
                            }
                        });
                    },
                ) {
                    Ok(api) => {
                        *freenet_api.borrow_mut() = Some(api);
                    }
                    Err(error) => {
                        connection_status.set(ConnectionStatus::Failed(error));
                    }
                }

                move || {
                    freenet_api.borrow_mut().take();
                }
            });
        }

        let board = BoardView::from(controller.visible_state());
        let outcome = controller.outcome();
        let left_table = controller.has_left_table();
        let session_active = controller.is_active();

        let can_roll = session_active
            && matches!(controller.state().status, GameStatus::InProgress)
            && controller.state().turn_phase == TurnPhase::AwaitingRoll;

        let can_pass = session_active && controller.must_pass();

        let can_resign =
            session_active && matches!(controller.state().status, GameStatus::InProgress);

        let active_name = player_name(board.active_player);

        let turn_text = if left_table {
            "Table left".to_owned()
        } else if outcome.is_some() {
            "Game complete".to_owned()
        } else if can_pass {
            format!("{active_name} must pass")
        } else {
            match board.turn_phase {
                TurnPhase::AwaitingRoll => format!("{active_name} to roll"),
                TurnPhase::Moving => format!("{active_name} is moving"),
            }
        };

        let legal_sources = if session_active && controller.state().turn_phase == TurnPhase::Moving
        {
            controller.legal_sources()
        } else {
            Vec::new()
        };

        let selected_source = if session_active {
            controller.selected_source()
        } else {
            None
        };

        let legal_destinations = if session_active && selected_source.is_some() {
            controller.legal_destinations().unwrap_or_default()
        } else {
            Vec::new()
        };

        let pending_role_check = load_pending_action(TEST_CONTRACT_ID);

        let role_selection_locked =
            selected_local_role.is_some() || !matches!(&pending_role_check, Ok(None));

        let on_select_white = {
            let local_role = local_role.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |_| match choose_local_role(Player::White) {
                Ok(()) => {
                    interface_error.set(None);
                    local_role.set(Ok(Some(Player::White)));
                }

                Err(error) => {
                    interface_error.set(Some(format!("White role could not be selected: {error}")));
                }
            })
        };

        let on_select_black = {
            let local_role = local_role.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |_| match choose_local_role(Player::Black) {
                Ok(()) => {
                    interface_error.set(None);
                    local_role.set(Ok(Some(Player::Black)));
                }

                Err(error) => {
                    interface_error.set(Some(format!("Black role could not be selected: {error}")));
                }
            })
        };

        let on_roll = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |_| {
                let mut next = (*controller).clone();

                match secure_local_dice() {
                    Ok(dice) => match next.begin_turn(dice) {
                        Ok(()) => {
                            interface_error.set(None);
                            controller.set(next);
                        }
                        Err(error) => {
                            interface_error
                                .set(Some(format!("The roll could not be applied: {error:?}")));
                        }
                    },
                    Err(error) => interface_error.set(Some(error)),
                }
            })
        };

        let on_pass = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |_| {
                let mut next = (*controller).clone();

                match next.pass_turn() {
                    Ok(()) => {
                        interface_error.set(None);
                        controller.set(next);
                    }
                    Err(error) => {
                        interface_error
                            .set(Some(format!("The turn could not be passed: {error:?}")));
                    }
                }
            })
        };

        let on_source = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |source: MoveSource| {
                let mut next = (*controller).clone();

                match next.select_source(source) {
                    Ok(()) => {
                        interface_error.set(None);
                        controller.set(next);
                    }
                    Err(error) => {
                        interface_error
                            .set(Some(format!("That checker cannot be selected: {error:?}")));
                    }
                }
            })
        };

        let on_destination = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |destination: MoveTarget| {
                let mut next = (*controller).clone();

                match next.choose_destination(destination) {
                    Ok(_) => {
                        interface_error.set(None);
                        controller.set(next);
                    }
                    Err(error) => {
                        interface_error
                            .set(Some(format!("That destination is not legal: {error:?}")));
                    }
                }
            })
        };

        let on_resign = {
            let pending_confirmation = pending_confirmation.clone();

            Callback::from(move |_| {
                pending_confirmation.set(Some(PendingConfirmation::Resign));
            })
        };

        let on_new_game = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();
            let pending_confirmation = pending_confirmation.clone();

            Callback::from(move |_| {
                if controller.is_active() {
                    pending_confirmation.set(Some(PendingConfirmation::NewGame));
                } else {
                    let mut next = (*controller).clone();
                    next.new_game();

                    interface_error.set(None);
                    pending_confirmation.set(None);
                    controller.set(next);
                }
            })
        };

        let on_leave = {
            let pending_confirmation = pending_confirmation.clone();

            Callback::from(move |_| {
                pending_confirmation.set(Some(PendingConfirmation::Leave));
            })
        };

        let on_cancel_confirmation = {
            let pending_confirmation = pending_confirmation.clone();

            Callback::from(move |_| {
                pending_confirmation.set(None);
            })
        };

        let on_confirm_action = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();
            let pending_confirmation = pending_confirmation.clone();

            Callback::from(move |_| {
                let Some(action) = *pending_confirmation else {
                    return;
                };

                let mut next = (*controller).clone();

                let result = match action {
                    PendingConfirmation::Resign => next.resign(),
                    PendingConfirmation::Leave => {
                        next.leave_table();
                        Ok(())
                    }
                    PendingConfirmation::NewGame => {
                        next.new_game();
                        Ok(())
                    }
                };

                match result {
                    Ok(()) => {
                        interface_error.set(None);
                        pending_confirmation.set(None);
                        controller.set(next);
                    }
                    Err(error) => {
                        interface_error.set(Some(format!(
                            "The requested action could not be completed: {error:?}"
                        )));
                        pending_confirmation.set(None);
                    }
                }
            })
        };

        let confirmation_overlay = pending_confirmation.as_ref().map_or_else(
            || html! {},
            |action| {
                html! {
                    <div
                        class="game-overlay confirmation-overlay"
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="confirmation-title"
                        aria-describedby="confirmation-message"
                    >
                        <div class="result-card confirmation-card">
                            <p class="result-kicker">{ "CONFIRM ACTION" }</p>

                            <h2 id="confirmation-title">
                                { action.title() }
                            </h2>

                            <p
                                id="confirmation-message"
                                class="confirmation-message"
                            >
                                { action.message() }
                            </p>

                            <div class="confirmation-actions">
                                <button
                                    type="button"
                                    class="confirmation-cancel"
                                    onclick={on_cancel_confirmation.clone()}
                                >
                                    { "Cancel" }
                                </button>

                                <button
                                    type="button"
                                    class="confirmation-danger"
                                    onclick={on_confirm_action.clone()}
                                >
                                    { action.confirm_label() }
                                </button>
                            </div>
                        </div>
                    </div>
                }
            },
        );

        let on_reconnect = {
            let selected_local_role = selected_local_role;
            let connection_status = connection_status.clone();
            let contract_status = contract_status.clone();
            let subscription_status = subscription_status.clone();
            let freenet_api = freenet_api.clone();
            let first_delta_submitted = first_delta_submitted.clone();
            let local_commitment_submitted = local_commitment_submitted.clone();
            let local_dice_secret = local_dice_secret.clone();
            let dice_secret_status = dice_secret_status.clone();

            Callback::from(move |_| {
                freenet_api.borrow_mut().take();

                /*
                 * Permit one submission attempt on the new connection.
                 * The durable pending action itself remains unchanged.
                 */
                *local_commitment_submitted.borrow_mut() = false;

                contract_status.set(ContractProbeStatus::WaitingForConnection);
                subscription_status.set(SubscriptionStatus::Pending);
                dice_secret_status.set("Checking browser storage".to_owned());

                let status_for_callback = connection_status.clone();
                let contract_for_response = contract_status.clone();
                let subscription_for_response = subscription_status.clone();
                let subscription_for_status = subscription_status.clone();
                let api_for_response = freenet_api.clone();
                let submitted_for_response = first_delta_submitted.clone();
                let commitment_for_response = local_commitment_submitted.clone();
                let secret_for_response = local_dice_secret.clone();
                let secret_status_for_response = dice_secret_status.clone();

                let api_for_open = freenet_api.clone();
                let connection_for_open = connection_status.clone();
                let contract_for_open = contract_status.clone();
                let subscription_for_open = subscription_status.clone();

                match connect(
                    move |status| {
                        match &status {
                            ConnectionStatus::Connecting => {
                                subscription_for_status.set(SubscriptionStatus::Pending);
                            }
                            ConnectionStatus::Connected => {}
                            ConnectionStatus::Disconnected | ConnectionStatus::Failed(_) => {
                                subscription_for_status.set(SubscriptionStatus::Inactive);
                            }
                        }

                        status_for_callback.set(status);
                    },
                    move |response| {
                        if let Some(classified) = classify_response(response) {
                            let ClassifiedResponse {
                                contract_status,
                                subscription_status,
                                contract_key,
                                authoritative_state,
                                should_submit_first_delta,
                            } = classified;

                            if let Some(contract) = contract_status {
                                contract_for_response.set(contract);
                            }

                            if let Some(subscription) = subscription_status {
                                subscription_for_response.set(subscription);
                            }

                            if should_submit_first_delta {
                                let Some(key) = contract_key else {
                                    contract_for_response.set(ContractProbeStatus::Failed(
                                        "The empty ledger response did not include a full contract key."
                                            .to_owned(),
                                    ));
                                    return;
                                };

                                {
                                    let mut submitted = submitted_for_response.borrow_mut();

                                    if *submitted {
                                        return;
                                    }

                                    *submitted = true;
                                }

                                contract_for_response.set(ContractProbeStatus::Updating);

                                let api_for_update = api_for_response.clone();
                                let contract_for_update = contract_for_response.clone();
                                let submitted_for_update = submitted_for_response.clone();

                                wasm_bindgen_futures::spawn_local(async move {
                                    let submit_result = {
                                        let mut api = api_for_update.borrow_mut();

                                        match api.as_mut() {
                                            Some(api) => submit_first_create_delta(api, key).await,
                                            None => {
                                                Err("Freenet connection closed before the update."
                                                    .to_owned())
                                            }
                                        }
                                    };

                                    match submit_result {
                                        Ok(()) => {
                                            contract_for_update
                                                .set(ContractProbeStatus::VerifyingUpdate);

                                            gloo_timers::future::TimeoutFuture::new(750).await;

                                            let refresh_result = {
                                                let mut api = api_for_update.borrow_mut();

                                                match api.as_mut() {
                                                    Some(api) => request_test_contract(api).await,
                                                    None => Err(
                                                        "Freenet connection closed before update verification."
                                                            .to_owned(),
                                                    ),
                                                }
                                            };

                                            if let Err(error) = refresh_result {
                                                contract_for_update
                                                    .set(ContractProbeStatus::Failed(error));
                                            }
                                        }
                                        Err(error) => {
                                            *submitted_for_update.borrow_mut() = false;
                                            contract_for_update
                                                .set(ContractProbeStatus::Failed(error));
                                        }
                                    }
                                });
                            }
                            if let (Some(key), Some(state_bytes)) =
                                (contract_key, authoritative_state)
                            {
                                let Some(local_player) = selected_local_role else {
                                    secret_status_for_response
                                        .set("Select White or Black to join the game".to_owned());
                                    return;
                                };

                                let plan = match plan_browser_commitment(&state_bytes, local_player)
                                {
                                    Ok(plan) => plan,
                                    Err(error) => {
                                        secret_status_for_response
                                            .set(format!("Recovery failed: {error}"));

                                        contract_for_response
                                            .set(ContractProbeStatus::Failed(error));

                                        return;
                                    }
                                };

                                match plan {
                                    CommitmentPlan::NoAction => {}

                                    CommitmentPlan::Accepted { secret } => {
                                        *secret_for_response.borrow_mut() = Some(secret);
                                        *commitment_for_response.borrow_mut() = true;

                                        secret_status_for_response.set(
                                            "Recovered and matched accepted commitment".to_owned(),
                                        );
                                    }

                                    CommitmentPlan::Submit {
                                        secret,
                                        pending,
                                        recovered_pending,
                                    } => {
                                        let delta = pending.delta;
                                        {
                                            let mut submitted =
                                                commitment_for_response.borrow_mut();

                                            if *submitted {
                                                return;
                                            }

                                            *submitted = true;
                                        }

                                        *secret_for_response.borrow_mut() = Some(secret);

                                        secret_status_for_response.set(if recovered_pending {
                                            "Recovered exact pending action; retrying".to_owned()
                                        } else {
                                            "Stored locally; awaiting network verification"
                                                .to_owned()
                                        });

                                        contract_for_response.set(ContractProbeStatus::Updating);

                                        let api_for_update = api_for_response.clone();
                                        let contract_for_update = contract_for_response.clone();

                                        wasm_bindgen_futures::spawn_local(async move {
                                            let submit_result = {
                                                let mut api = api_for_update.borrow_mut();

                                                match api.as_mut() {
                                                    Some(api) => {
                                                        submit_action_delta(
                                                            api,
                                                            key,
                                                            delta,
                                                        )
                                                        .await
                                                    }
                                                    None => Err(
                                                        "Freenet connection closed before the pending commitment update."
                                                            .to_owned(),
                                                    ),
                                                }
                                            };

                                            match submit_result {
                                                Ok(()) => {
                                                    contract_for_update
                                                        .set(ContractProbeStatus::VerifyingUpdate);

                                                    gloo_timers::future::TimeoutFuture::new(750)
                                                        .await;

                                                    let refresh_result = {
                                                        let mut api = api_for_update.borrow_mut();

                                                        match api.as_mut() {
                                                            Some(api) => {
                                                                request_test_contract(api)
                                                                    .await
                                                            }
                                                            None => Err(
                                                                "Freenet connection closed before commitment verification."
                                                                    .to_owned(),
                                                            ),
                                                        }
                                                    };

                                                    if let Err(error) = refresh_result {
                                                        contract_for_update.set(
                                                            ContractProbeStatus::Failed(error),
                                                        );
                                                    }
                                                }

                                                Err(error) => {
                                                    /*
                                                     * Do not clear the durable pending record
                                                     * and do not regenerate the action. A later
                                                     * reconnect will reconcile and retry these
                                                     * exact stored bytes.
                                                     */
                                                    contract_for_update
                                                        .set(ContractProbeStatus::Failed(error));
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    },
                    move || {
                        let api_for_request = api_for_open.clone();
                        let connection_for_request = connection_for_open.clone();
                        let contract_for_request = contract_for_open.clone();
                        let subscription_for_request = subscription_for_open.clone();

                        wasm_bindgen_futures::spawn_local(async move {
                            contract_for_request.set(ContractProbeStatus::Requesting);

                            subscription_for_request.set(SubscriptionStatus::Pending);

                            let result = {
                                let mut api = api_for_request.borrow_mut();

                                match api.as_mut() {
                                    Some(api) => request_test_contract(api).await,
                                    None => Err(
                                        "Freenet WebSocket opened without an active API handle."
                                            .to_owned(),
                                    ),
                                }
                            };

                            if let Err(error) = result {
                                connection_for_request.set(ConnectionStatus::Failed(error.clone()));

                                contract_for_request.set(ContractProbeStatus::Failed(error));

                                subscription_for_request.set(SubscriptionStatus::Inactive);
                            }
                        });
                    },
                ) {
                    Ok(api) => {
                        *freenet_api.borrow_mut() = Some(api);
                    }
                    Err(error) => {
                        connection_status.set(ConnectionStatus::Failed(error));
                    }
                }
            })
        };

        let terminal_overlay = if left_table {
            html! {
                <div class="game-overlay" role="dialog" aria-modal="true">
                    <div class="result-card leave-card">
                        <p class="result-kicker">{ "LOCAL SESSION" }</p>
                        <h2>{ "You left the table" }</h2>
                        <p>{ "The previous local session has ended." }</p>

                        <button
                            type="button"
                            class="overlay-action"
                            onclick={on_new_game.clone()}
                        >
                            { "Start new game" }
                        </button>
                    </div>
                </div>
            }
        } else {
            outcome.map_or_else(
                || html! {},
                |game_outcome| {
                    let (winner, title, subtitle) = match game_outcome {
                        LocalGameOutcome::Completed { winner, points } => (
                            winner,
                            format!("{} wins!", player_name(winner)),
                            format!(
                                "{} — {} point{}",
                                result_name(points),
                                points,
                                if points == 1 { "" } else { "s" }
                            ),
                        ),
                        LocalGameOutcome::Resigned { resigned, winner } => (
                            winner,
                            format!("{} wins!", player_name(winner)),
                            format!("{} resigned", player_name(resigned)),
                        ),
                    };

                    let winner_class = match winner {
                        Player::White => "winner-white",
                        Player::Black => "winner-black",
                    };

                    html! {
                        <div class="game-overlay" role="dialog" aria-modal="true">
                            <div class="celebration" aria-hidden="true">
                                {
                                    for (0..24).map(|index| {
                                        html! {
                                            <span
                                                class={classes!(
                                                    "confetti-piece",
                                                    format!("confetti-{}", index + 1),
                                                )}
                                            ></span>
                                        }
                                    })
                                }
                            </div>

                            <div class={classes!("result-card", winner_class)}>
                                <p class="result-kicker">{ "GAME COMPLETE" }</p>
                                <div class="result-emblem" aria-hidden="true">{ "★" }</div>
                                <h2>{ title }</h2>
                                <p class="result-subtitle">{ subtitle }</p>

                                <button
                                    type="button"
                                    class="overlay-action"
                                    onclick={on_new_game.clone()}
                                >
                                    { "Play again" }
                                </button>
                            </div>
                        </div>
                    }
                },
            )
        };

        html! {
            <main class="app-shell">
                <header class="app-header">
                    <div>
                        <p class="mode-label">{ "LOCAL TWO-PLAYER MODE" }</p>
                        <h1>{ "Freenet Backgammon" }</h1>
                    </div>

                    <div
                        class={classes!(
                            "connection-badge",
                            connection_status.css_class(),
                        )}
                        role="status"
                    >
                        <span class="connection-dot" aria-hidden="true"></span>
                        { connection_status.label() }
                    </div>
                </header>

                <section class="game-layout">
                    <aside class="left-rail">
                        <PlayerPanel
                            player={Player::Black}
                            name={"Player Two".to_owned()}
                            score={0}
                            active={
                                session_active
                                    && board.active_player == Player::Black
                            }
                            bar={board.black_bar}
                            borne_off={board.black_borne_off}
                        />

                        <PlayerPanel
                            player={Player::White}
                            name={"Player One".to_owned()}
                            score={0}
                            active={
                                session_active
                                    && board.active_player == Player::White
                            }
                            bar={board.white_bar}
                            borne_off={board.white_borne_off}
                        />

                        <section class="panel turn-panel" aria-labelledby="turn-heading">
                            <h2 id="turn-heading">{ "Turn" }</h2>
                            <strong>{ turn_text }</strong>

                            <p class="panel-note">
                                { controller.status_message().to_owned() }
                            </p>

                            {
                                interface_error.as_ref().map_or_else(
                                    || html! {},
                                    |error| html! {
                                        <p class="interface-error" role="alert">
                                            { error }
                                        </p>
                                    },
                                )
                            }
                        </section>

                        <DiceDisplay dice={board.dice} />

                        <GameControls
                            can_roll={can_roll}
                            can_pass={can_pass}
                            can_resign={can_resign}
                            can_leave={!left_table}
                            can_reconnect={connection_status.can_reconnect()}
                            on_roll={on_roll}
                            on_pass={on_pass}
                            on_resign={on_resign}
                            on_new_game={on_new_game.clone()}
                            on_reconnect={on_reconnect}
                            on_leave={on_leave}
                        />
                    </aside>

                    <section class="board-stage" aria-label="Game board">
                        <Board
                            board={board}
                            legal_sources={legal_sources}
                            selected_source={selected_source}
                            legal_destinations={legal_destinations}
                            on_source={on_source}
                            on_destination={on_destination}
                        />
                    </section>

                    <aside class="right-rail">
                        <MoveHistory history={controller.history().to_vec()} />

                        <section class="panel status-panel" aria-labelledby="status-heading">
                            <h2 id="status-heading">{ "Connection" }</h2>

                            <div
                                class="role-selector"
                                aria-label="Choose local player role"
                            >
                                <button
                                    type="button"
                                    class={classes!(
                                        "role-choice",
                                        (
                                            selected_local_role
                                                == Some(Player::White)
                                        )
                                            .then_some("selected"),
                                    )}
                                    aria-pressed={
                                        (
                                            selected_local_role
                                                == Some(Player::White)
                                        )
                                            .to_string()
                                    }
                                    disabled={role_selection_locked}
                                    onclick={on_select_white}
                                >
                                    { "Play as White" }
                                </button>

                                <button
                                    type="button"
                                    class={classes!(
                                        "role-choice",
                                        (
                                            selected_local_role
                                                == Some(Player::Black)
                                        )
                                            .then_some("selected"),
                                    )}
                                    aria-pressed={
                                        (
                                            selected_local_role
                                                == Some(Player::Black)
                                        )
                                            .to_string()
                                    }
                                    disabled={role_selection_locked}
                                    onclick={on_select_black}
                                >
                                    { "Play as Black" }
                                </button>
                            </div>

                            {
                                match &*local_role {
                                    Err(error) => html! {
                                        <p class="interface-error" role="alert">
                                            {
                                                format!(
                                                    "Stored local role failed validation: {error}"
                                                )
                                            }
                                        </p>
                                    },

                                    Ok(_) => html! {},
                                }
                            }

                            {
                                match &pending_role_check {
                                    Err(error) => html! {
                                        <p class="interface-error" role="alert">
                                            {
                                                format!(
                                                    "Pending-action role guard failed: {error}"
                                                )
                                            }
                                        </p>
                                    },

                                    Ok(_) => html! {},
                                }
                            }

                            <dl class="status-list">
                                <div>
                                    <dt>{ "Game mode" }</dt>
                                    <dd>{ "Network commitment test" }</dd>
                                </div>

                                <div class="role-status-row">
                                    <dt>{ "Local role" }</dt>
                                    <dd>
                                        {
                                            match &*local_role {
                                                Ok(Some(player)) => player_name(*player),
                                                Ok(None) => "Not selected",
                                                Err(_) => "Storage error",
                                            }
                                        }
                                    </dd>
                                </div>

                                <div>
                                    <dt>{ "Freenet" }</dt>
                                    <dd>{ connection_status.label() }</dd>
                                </div>

                                <div>
                                    <dt>{ "Network detail" }</dt>
                                    <dd>{ connection_status.detail() }</dd>
                                </div>

                                <div>
                                    <dt>{ "Contract" }</dt>
                                    <dd>{ contract_status.contract_label() }</dd>
                                </div>

                                <div>
                                    <dt>{ "Subscription" }</dt>
                                    <dd>{ subscription_status.label() }</dd>
                                </div>

                                <div>
                                    <dt>{ "State check" }</dt>
                                    <dd>{ contract_status.state_label() }</dd>
                                </div>

                                <div>
                                    <dt>{ "Dice secret" }</dt>
                                    <dd>{ (*dice_secret_status).clone() }</dd>
                                </div>

                                <div>
                                    <dt>{ "Game state" }</dt>
                                    <dd>
                                        {
                                            if left_table {
                                                "Table left"
                                            } else if outcome.is_some() {
                                                "Game complete"
                                            } else if can_pass {
                                                "Awaiting pass"
                                            } else if can_roll {
                                                "Ready to roll"
                                            } else {
                                                "Turn in progress"
                                            }
                                        }
                                    </dd>
                                </div>
                            </dl>
                        </section>
                    </aside>
                </section>

                { terminal_overlay }
                { confirmation_overlay }
            </main>
        }
    }

    pub fn run() {
        yew::Renderer::<App>::new().render();
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    browser::run();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!(
        "backgammon-client is a browser application; \
         build it for wasm32-unknown-unknown with Trunk."
    );
}
