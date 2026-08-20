#[cfg(target_arch = "wasm32")]
mod components;

pub mod challenge;
pub mod challenge_state;
pub mod commitment_planner;
pub mod controller;
pub mod genesis_handshake;
pub mod genesis_handshake_store;
pub mod ledger_codec;
pub mod lobby;
pub mod local_identity_store;
pub mod local_role_store;
pub mod pending_action;
pub mod pending_action_store;
pub mod play_turn_planner;
pub mod projection;
pub mod request_roll_planner;
pub mod reveal_planner;
pub mod secret_store;
pub mod transport;

#[cfg(test)]
mod test_support;

#[cfg(target_arch = "wasm32")]
mod browser {
    use backgammon_core::{GameState, MoveSource, MoveTarget, Player, TurnPhase, TurnSequence};
    use backgammon_protocol::{replay_game, DiceSecret, GameActionPayload};
    use yew::prelude::*;

    use crate::commitment_planner::{plan_commitment, CommitmentPlan, CommitmentPlannerInput};
    use crate::components::board::Board;
    use crate::components::controls::GameControls;
    use crate::components::dice::DiceDisplay;
    use crate::components::history::MoveHistory;
    use crate::components::player_panel::PlayerPanel;
    use crate::controller::{LocalGameController, LocalGameOutcome, LocalTurnRecord};
    use crate::ledger_codec::{decode_verified_ledger, decode_verified_replay};
    use crate::local_identity_store::{
        load_local_identity, load_or_create_local_identity, player_id_for_signing_key,
        role_for_player_id,
    };
    use crate::local_role_store::{load_local_role, store_local_role};
    use crate::pending_action_store::{
        load_pending_action, remove_pending_action, store_pending_action,
    };
    use crate::play_turn_planner::{plan_play_turn, PlayTurnPlan, PlayTurnPlannerInput};
    use crate::projection::BoardView;
    use crate::request_roll_planner::{
        plan_request_roll, RequestRollPlan, RequestRollPlannerInput,
    };
    use crate::reveal_planner::{plan_reveal, RevealPlan, RevealPlannerInput};
    use crate::secret_store::{load_dice_secret, store_dice_secret};
    use crate::transport::{
        classify_response, connect, request_test_contract, submit_action_delta, ClassifiedResponse,
        ConnectionStatus, ContractProbeStatus, SubscriptionStatus, TEST_CONTRACT_ID,
    };

    fn format_player_id(player_id: &[u8; 32]) -> String {
        player_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join("")
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
        let signing_key = load_local_identity()?
            .ok_or_else(|| "Local signing identity is unavailable.".to_owned())?;

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
            signing_key: &signing_key,
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum NetworkActionKind {
        Commitment,
        Reveal,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SecretlessNetworkActionKind {
        PlayTurn,
        RequestRoll,
    }

    enum BrowserNetworkActionPlan {
        NoAction,

        Accepted {
            secret: DiceSecret,
            kind: NetworkActionKind,
        },

        Submit {
            secret: DiceSecret,
            pending: crate::pending_action::PendingAction,
            recovered_pending: bool,
            kind: NetworkActionKind,
        },

        SecretlessAccepted {
            kind: SecretlessNetworkActionKind,
        },

        SecretlessSubmit {
            pending: crate::pending_action::PendingAction,
            recovered_pending: bool,
            kind: SecretlessNetworkActionKind,
        },
    }

    fn plan_browser_reveal(
        authoritative_state: &[u8],
        local_player: Player,
    ) -> Result<RevealPlan, String> {
        let signing_key = load_local_identity()?
            .ok_or_else(|| "Local signing identity is unavailable.".to_owned())?;

        let pending = load_pending_action(TEST_CONTRACT_ID)?;

        let stored_secret = if let Some(pending) = pending.as_ref() {
            let record = pending.verify()?;

            let GameActionPayload::RevealDice { turn, player, .. } = &record.payload else {
                return Err("Stored pending action is not a dice reveal.".to_owned());
            };

            if *player != local_player {
                return Err("Stored pending reveal belongs to a different local player.".to_owned());
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
         * A fresh action ID is candidate material only. It is unavailable while
         * an exact durable pending action exists, so retry cannot regenerate it.
         */
        let new_action_id = if pending.is_none() && stored_secret.is_some() {
            Some(secure_random_32("network action ID")?)
        } else {
            None
        };

        let plan = plan_reveal(RevealPlannerInput {
            contract_id: TEST_CONTRACT_ID,
            local_player,
            signing_key: &signing_key,
            authoritative_state,
            pending: pending.as_ref(),
            stored_secret,
            new_action_id,
        })?;

        match &plan {
            RevealPlan::NoAction => {}

            RevealPlan::Accepted { .. } => {
                if pending.is_some() {
                    remove_pending_action(TEST_CONTRACT_ID)?;
                }
            }

            RevealPlan::Submit {
                pending,
                recovered_pending,
                ..
            } => {
                if !recovered_pending {
                    let record = pending.verify()?;

                    let GameActionPayload::RevealDice { player, .. } = record.payload else {
                        return Err("New reveal plan produced a non-reveal action.".to_owned());
                    };

                    if player != local_player {
                        return Err(
                            "New reveal plan produced an action for another player.".to_owned()
                        );
                    }

                    /*
                     * The commitment path already persisted the secret. Preserve
                     * it through reveal acceptance for restart verification.
                     */
                    store_pending_action(pending)?;
                }
            }
        }

        Ok(plan)
    }

    fn map_reveal_plan(plan: RevealPlan) -> BrowserNetworkActionPlan {
        match plan {
            RevealPlan::NoAction => BrowserNetworkActionPlan::NoAction,

            RevealPlan::Accepted { secret } => BrowserNetworkActionPlan::Accepted {
                secret,
                kind: NetworkActionKind::Reveal,
            },

            RevealPlan::Submit {
                secret,
                pending,
                recovered_pending,
            } => BrowserNetworkActionPlan::Submit {
                secret,
                pending,
                recovered_pending,
                kind: NetworkActionKind::Reveal,
            },
        }
    }

    fn plan_browser_play_turn(
        authoritative_state: &[u8],
        local_player: Player,
        sequence: Option<&TurnSequence>,
    ) -> Result<PlayTurnPlan, String> {
        let signing_key = load_local_identity()?
            .ok_or_else(|| "Local signing identity is unavailable.".to_owned())?;

        let pending = load_pending_action(TEST_CONTRACT_ID)?;

        if let Some(pending) = pending.as_ref() {
            let record = pending.verify()?;

            let GameActionPayload::PlayTurn { player, .. } = &record.payload else {
                return Err("Stored pending action is not a completed game turn.".to_owned());
            };

            if *player != local_player {
                return Err("Stored pending turn belongs to a different local player.".to_owned());
            }
        }

        /*
         * Fresh entropy is supplied only when the interface has completed a
         * sequence and no exact durable pending action already exists.
         */
        let new_action_id = if pending.is_none() && sequence.is_some() {
            Some(secure_random_32("network turn action ID")?)
        } else {
            None
        };

        let plan = plan_play_turn(PlayTurnPlannerInput {
            contract_id: TEST_CONTRACT_ID,
            local_player,
            signing_key: &signing_key,
            authoritative_state,
            pending: pending.as_ref(),
            sequence,
            new_action_id,
        })?;

        match &plan {
            PlayTurnPlan::NoAction => {}

            PlayTurnPlan::Accepted => {
                if pending.is_some() {
                    remove_pending_action(TEST_CONTRACT_ID)?;
                }
            }

            PlayTurnPlan::Submit {
                pending,
                recovered_pending,
            } => {
                if !recovered_pending {
                    let record = pending.verify()?;

                    let GameActionPayload::PlayTurn { player, .. } = record.payload else {
                        return Err("New turn plan produced a non-turn action.".to_owned());
                    };

                    if player != local_player {
                        return Err(
                            "New turn plan produced an action for another player.".to_owned()
                        );
                    }

                    /*
                     * Persist the exact encoded turn before any network
                     * submission. Reload and reconnect must retry these same
                     * bytes rather than rebuilding the action.
                     */
                    store_pending_action(pending)?;
                }
            }
        }

        Ok(plan)
    }

    fn map_play_turn_plan(plan: PlayTurnPlan) -> BrowserNetworkActionPlan {
        match plan {
            PlayTurnPlan::NoAction => BrowserNetworkActionPlan::NoAction,

            PlayTurnPlan::Accepted => BrowserNetworkActionPlan::SecretlessAccepted {
                kind: SecretlessNetworkActionKind::PlayTurn,
            },

            PlayTurnPlan::Submit {
                pending,
                recovered_pending,
            } => BrowserNetworkActionPlan::SecretlessSubmit {
                pending,
                recovered_pending,
                kind: SecretlessNetworkActionKind::PlayTurn,
            },
        }
    }

    fn plan_browser_request_roll(
        authoritative_state: &[u8],
        local_player: Player,
        requested: bool,
    ) -> Result<RequestRollPlan, String> {
        let signing_key = load_local_identity()?
            .ok_or_else(|| "Local signing identity is unavailable.".to_owned())?;

        let pending = load_pending_action(TEST_CONTRACT_ID)?;

        if let Some(pending) = pending.as_ref() {
            let record = pending.verify()?;

            let GameActionPayload::RequestRoll { player, .. } = &record.payload else {
                return Err("Stored pending action is not a roll request.".to_owned());
            };

            if *player != local_player {
                return Err(
                    "Stored pending roll request belongs to a different local player.".to_owned(),
                );
            }
        }

        /*
         * Fresh entropy is candidate material only for an explicit human request.
         * Recovery of an exact durable pending RequestRoll never regenerates it.
         */
        let new_action_id = if pending.is_none() && requested {
            Some(secure_random_32("network roll-request action ID")?)
        } else {
            None
        };

        let plan = plan_request_roll(RequestRollPlannerInput {
            contract_id: TEST_CONTRACT_ID,
            local_player,
            signing_key: &signing_key,
            authoritative_state,
            pending: pending.as_ref(),
            requested,
            new_action_id,
        })?;

        match &plan {
            RequestRollPlan::NoAction => {}

            RequestRollPlan::Accepted => {
                if pending.is_some() {
                    remove_pending_action(TEST_CONTRACT_ID)?;
                }
            }

            RequestRollPlan::Submit {
                pending,
                recovered_pending,
            } => {
                if !recovered_pending {
                    let record = pending.verify()?;

                    let GameActionPayload::RequestRoll { player, .. } = record.payload else {
                        return Err("New roll-request plan produced a non-roll action.".to_owned());
                    };

                    if player != local_player {
                        return Err(
                            "New roll-request plan produced an action for another player."
                                .to_owned(),
                        );
                    }

                    /*
                     * Persist the exact encoded request before any network submission.
                     * Reload and reconnect must retry these same bytes.
                     */
                    store_pending_action(pending)?;
                }
            }
        }

        Ok(plan)
    }

    fn map_request_roll_plan(plan: RequestRollPlan) -> BrowserNetworkActionPlan {
        match plan {
            RequestRollPlan::NoAction => BrowserNetworkActionPlan::NoAction,

            RequestRollPlan::Accepted => BrowserNetworkActionPlan::SecretlessAccepted {
                kind: SecretlessNetworkActionKind::RequestRoll,
            },

            RequestRollPlan::Submit {
                pending,
                recovered_pending,
            } => BrowserNetworkActionPlan::SecretlessSubmit {
                pending,
                recovered_pending,
                kind: SecretlessNetworkActionKind::RequestRoll,
            },
        }
    }

    fn plan_browser_network_action(
        authoritative_state: &[u8],
        local_player: Player,
    ) -> Result<BrowserNetworkActionPlan, String> {
        if let Some(pending) = load_pending_action(TEST_CONTRACT_ID)? {
            let record = pending.verify()?;

            return match record.payload {
                GameActionPayload::CommitDice { .. } => {
                    match plan_browser_commitment(authoritative_state, local_player)? {
                        CommitmentPlan::NoAction => Ok(BrowserNetworkActionPlan::NoAction),

                        CommitmentPlan::Accepted { secret } => {
                            /*
                             * Acceptance removes the pending commitment. Continue
                             * immediately into reveal planning so the browser does
                             * not require an unrelated later update to advance.
                             */
                            match plan_browser_reveal(authoritative_state, local_player)? {
                                RevealPlan::NoAction => Ok(BrowserNetworkActionPlan::Accepted {
                                    secret,
                                    kind: NetworkActionKind::Commitment,
                                }),

                                reveal => Ok(map_reveal_plan(reveal)),
                            }
                        }

                        CommitmentPlan::Submit {
                            secret,
                            pending,
                            recovered_pending,
                        } => Ok(BrowserNetworkActionPlan::Submit {
                            secret,
                            pending,
                            recovered_pending,
                            kind: NetworkActionKind::Commitment,
                        }),
                    }
                }

                GameActionPayload::RevealDice { .. } => Ok(map_reveal_plan(plan_browser_reveal(
                    authoritative_state,
                    local_player,
                )?)),

                GameActionPayload::RequestRoll { .. } => Ok(map_request_roll_plan(
                    plan_browser_request_roll(authoritative_state, local_player, false)?,
                )),

                GameActionPayload::PlayTurn { .. } => Ok(map_play_turn_plan(
                    plan_browser_play_turn(authoritative_state, local_player, None)?,
                )),

                _ => Err(
                    "Stored pending action is not supported by the browser network-action loop."
                        .to_owned(),
                ),
            };
        }

        match plan_browser_commitment(authoritative_state, local_player)? {
            CommitmentPlan::NoAction => Ok(map_reveal_plan(plan_browser_reveal(
                authoritative_state,
                local_player,
            )?)),

            CommitmentPlan::Accepted { secret } => {
                match plan_browser_reveal(authoritative_state, local_player)? {
                    RevealPlan::NoAction => Ok(BrowserNetworkActionPlan::Accepted {
                        secret,
                        kind: NetworkActionKind::Commitment,
                    }),

                    reveal => Ok(map_reveal_plan(reveal)),
                }
            }

            CommitmentPlan::Submit {
                secret,
                pending,
                recovered_pending,
            } => Ok(BrowserNetworkActionPlan::Submit {
                secret,
                pending,
                recovered_pending,
                kind: NetworkActionKind::Commitment,
            }),
        }
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

    fn authoritative_game_projection(
        state_bytes: &[u8],
    ) -> Result<Option<(GameState, Vec<LocalTurnRecord>)>, String> {
        let ledger = decode_verified_ledger(state_bytes)?;

        /*
         * An empty ledger has no CreateGame action and therefore cannot yet
         * produce a ReplayedGame. The first-create submission path handles it.
         */
        if ledger.action_count() == 0 {
            return Ok(None);
        }

        let replay = decode_verified_replay(state_bytes)?;

        let history = replay
            .completed_turns
            .into_iter()
            .map(|turn| LocalTurnRecord {
                player: turn.player,
                dice: turn.dice,
                moves: turn.sequence.moves,
            })
            .collect();

        Ok(Some((replay.state, history)))
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
        let local_network_action_submitted = use_mut_ref(|| None::<[u8; 32]>);
        let local_dice_secret = use_mut_ref(|| None::<DiceSecret>);
        let dice_secret_status = use_state(|| "Checking browser storage".to_owned());

        /* Passive cryptographic identity; not yet used for role selection. */
        let local_identity_status = use_state(|| "Checking local identity".to_owned());
        let local_player_id = use_state(|| None::<[u8; 32]>);

        /*
         * Role derived from this persistent PlayerId and a verified
         * authoritative GameConfiguration. None means no verified role
         * decision is available yet; Some(None) means not a participant.
         */
        let authoritative_local_role = use_state(|| None::<Option<Player>>);

        /*
         * User-triggered PlayTurn submission needs the exact full ContractKey
         * and verified parent bytes from the latest GetResponse. Subscription
         * notifications alone do not provide both values.
         */
        let latest_contract_key = use_mut_ref(|| None::<freenet_stdlib::prelude::ContractKey>);
        let latest_authoritative_state = use_mut_ref(|| None::<Vec<u8>>);

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

        /*
         * Read-only comparison between this browser's persistent identity and
         * the participant IDs recorded in the verified authoritative game.
         * This does not yet control the temporary local role selector.
         */
        let local_player_id_text = match *local_player_id {
            Some(player_id) => format_player_id(&player_id),
            None => "Identity unavailable".to_owned(),
        };

        let authoritative_identity_role_text = match *local_player_id {
            None => "Identity unavailable".to_owned(),
            Some(_) => match *authoritative_local_role {
                None => "Waiting for game state".to_owned(),
                Some(Some(Player::White)) => "White".to_owned(),
                Some(Some(Player::Black)) => "Black".to_owned(),
                Some(None) => "Not a participant".to_owned(),
            },
        };
        let authoritative_player_role = (*authoritative_local_role).flatten();

        {
            let identity_status = local_identity_status.clone();
            let player_id = local_player_id.clone();

            use_effect_with((), move |_| {
                match secure_random_32("local identity seed")
                    .and_then(load_or_create_local_identity)
                {
                    Ok(identity) => {
                        player_id.set(Some(player_id_for_signing_key(&identity)));
                        identity_status.set("Local identity ready".to_owned());
                    }
                    Err(error) => {
                        player_id.set(None);
                        identity_status.set(format!("Local identity error: {error}"));
                    }
                }

                || {}
            });
        }

        {
            let latest_authoritative_state = latest_authoritative_state.clone();
            let authoritative_local_role = authoritative_local_role.clone();

            use_effect_with(*local_player_id, move |player_id| {
                let resolved_role = match *player_id {
                    None => None,
                    Some(player_id) => {
                        let state = latest_authoritative_state.borrow();

                        state.as_ref().and_then(|state_bytes| {
                            decode_verified_replay(state_bytes)
                                .ok()
                                .map(|replay| role_for_player_id(&replay.configuration, &player_id))
                        })
                    }
                };

                authoritative_local_role.set(resolved_role);

                || {}
            });
        }

        {
            let connection_status = connection_status.clone();
            let contract_status = contract_status.clone();
            let subscription_status = subscription_status.clone();
            let freenet_api = freenet_api.clone();
            let local_network_action_submitted = local_network_action_submitted.clone();
            let local_dice_secret = local_dice_secret.clone();
            let dice_secret_status = dice_secret_status.clone();
            let latest_contract_key = latest_contract_key.clone();
            let latest_authoritative_state = latest_authoritative_state.clone();
            let local_player_id_for_effect = local_player_id.clone();
            let authoritative_local_role_for_effect = authoritative_local_role.clone();
            let controller_for_effect = controller.clone();

            use_effect_with((), move |_| {
                /*
                 * Replacing the role dependency replaces the active transport
                 * closure. Any durable pending action remains in storage.
                 */
                freenet_api.borrow_mut().take();
                *local_network_action_submitted.borrow_mut() = None;
                *local_dice_secret.borrow_mut() = None;
                latest_contract_key.borrow_mut().take();
                latest_authoritative_state.borrow_mut().take();
                authoritative_local_role_for_effect.set(None);

                let status_for_callback = connection_status.clone();
                let contract_for_response = contract_status.clone();
                let contract_for_host_error = contract_status.clone();
                let subscription_for_response = subscription_status.clone();
                let subscription_for_status = subscription_status.clone();
                let api_for_response = freenet_api.clone();
                let network_action_for_response = local_network_action_submitted.clone();
                let secret_for_response = local_dice_secret.clone();
                let secret_status_for_response = dice_secret_status.clone();
                let key_for_response = latest_contract_key.clone();
                let state_for_response = latest_authoritative_state.clone();
                let player_id_for_response = local_player_id_for_effect.clone();
                let authoritative_role_for_response = authoritative_local_role_for_effect.clone();
                let controller_for_response = controller_for_effect.clone();

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
                            } = classified;

                            if let Some(contract) = contract_status {
                                contract_for_response.set(contract);
                            }

                            if let Some(subscription) = subscription_status {
                                subscription_for_response.set(subscription);
                            }

                            if let (Some(key), Some(state_bytes)) =
                                (contract_key, authoritative_state)
                            {
                                let authoritative_state =
                                    match authoritative_game_projection(&state_bytes) {
                                        Ok(state) => state,

                                        Err(error) => {
                                            contract_for_response.set(ContractProbeStatus::Failed(
                                                format!("Authoritative replay failed: {error}"),
                                            ));

                                            return;
                                        }
                                    };

                                /*
                                 * An unchanged parent ledger must not erase a
                                 * local checker preview while a turn is being
                                 * prepared or awaiting authoritative acceptance.
                                 */
                                let state_changed =
                                    state_for_response.borrow().as_ref() != Some(&state_bytes);

                                /*
                                 * Cache only after complete ledger decoding and
                                 * deterministic protocol replay have succeeded.
                                 */
                                *key_for_response.borrow_mut() = Some(key.clone());
                                *state_for_response.borrow_mut() = Some(state_bytes.clone());

                                let resolved_role = match *player_id_for_response {
                                    None => None,
                                    Some(player_id) => {
                                        decode_verified_replay(&state_bytes).ok().map(|replay| {
                                            role_for_player_id(&replay.configuration, &player_id)
                                        })
                                    }
                                };

                                let authoritative_player = resolved_role.flatten();
                                authoritative_role_for_response.set(resolved_role);

                                if state_changed {
                                    if let Some((authoritative_state, authoritative_history)) =
                                        authoritative_state
                                    {
                                        let mut next = (*controller_for_response).clone();

                                        if let Err(error) = next
                                            .sync_authoritative_state_and_history(
                                                authoritative_state,
                                                authoritative_history,
                                            )
                                        {
                                            contract_for_response.set(
                                                ContractProbeStatus::Failed(
                                                    format!(
                                                        "Authoritative board synchronization failed: {error:?}"
                                                    ),
                                                ),
                                            );

                                            return;
                                        }

                                        controller_for_response.set(next);
                                    }
                                }

                                let Some(local_player) = authoritative_player else {
                                    secret_status_for_response
                                        .set("This browser identity is not an authoritative game participant".to_owned());
                                    return;
                                };

                                let plan =
                                    match plan_browser_network_action(&state_bytes, local_player) {
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
                                    BrowserNetworkActionPlan::NoAction => {
                                        secret_status_for_response
                                            .set("No dice-secret recovery needed".to_owned());
                                    }

                                    BrowserNetworkActionPlan::Accepted { secret, kind } => {
                                        *secret_for_response.borrow_mut() = Some(secret);
                                        *network_action_for_response.borrow_mut() = None;

                                        secret_status_for_response.set(match kind {
                                            NetworkActionKind::Commitment => {
                                                "Recovered and matched accepted commitment"
                                                    .to_owned()
                                            }

                                            NetworkActionKind::Reveal => {
                                                "Recovered and matched accepted reveal".to_owned()
                                            }
                                        });
                                    }

                                    BrowserNetworkActionPlan::SecretlessAccepted { kind: _ } => {
                                        /*
                                         * The authoritative board was already
                                         * synchronized above from the accepted
                                         * history. Only the local submission
                                         * guard remains to be cleared.
                                         */
                                        *network_action_for_response.borrow_mut() = None;
                                    }

                                    BrowserNetworkActionPlan::SecretlessSubmit {
                                        pending,
                                        recovered_pending,
                                        kind: _,
                                    } => {
                                        let action_id = pending.action_id;
                                        let delta = pending.delta;

                                        {
                                            let mut submitted =
                                                network_action_for_response.borrow_mut();

                                            if submitted.as_ref() == Some(&action_id) {
                                                return;
                                            }

                                            *submitted = Some(action_id);
                                        }

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
                                                            "Freenet connection closed before the pending turn update."
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
                                                                    "Freenet connection closed before turn verification."
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
                                                     * Keep the exact pending
                                                     * turn. Reconnect will
                                                     * retry the same bytes.
                                                     */
                                                    contract_for_update.set(
                                                        ContractProbeStatus::Failed(format!(
                                                            "{}{}",
                                                            if recovered_pending {
                                                                "Recovered turn retry failed: "
                                                            } else {
                                                                "Turn submission failed: "
                                                            },
                                                            error,
                                                        )),
                                                    );
                                                }
                                            }
                                        });
                                    }

                                    BrowserNetworkActionPlan::Submit {
                                        secret,
                                        pending,
                                        recovered_pending,
                                        kind,
                                    } => {
                                        let action_id = pending.action_id;
                                        let delta = pending.delta;

                                        {
                                            let mut submitted =
                                                network_action_for_response.borrow_mut();

                                            if submitted.as_ref() == Some(&action_id) {
                                                return;
                                            }

                                            *submitted = Some(action_id);
                                        }

                                        *secret_for_response.borrow_mut() = Some(secret);

                                        let action_name = match kind {
                                            NetworkActionKind::Commitment => "commitment",
                                            NetworkActionKind::Reveal => "reveal",
                                        };

                                        secret_status_for_response.set(if recovered_pending {
                                            format!(
                                                "Recovered exact pending {action_name}; retrying"
                                            )
                                        } else {
                                            format!(
                                                "Stored {action_name} locally; awaiting network verification"
                                            )
                                        });

                                        contract_for_response.set(ContractProbeStatus::Updating);

                                        let api_for_update = api_for_response.clone();
                                        let contract_for_update = contract_for_response.clone();

                                        wasm_bindgen_futures::spawn_local(async move {
                                            let submit_result = {
                                                let mut api = api_for_update.borrow_mut();

                                                match api.as_mut() {
                                                    Some(api) => {
                                                        submit_action_delta(api, key, delta).await
                                                    }

                                                    None => Err(format!(
                                                        "Freenet connection closed before the pending {action_name} update."
                                                    )),
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
                                                                request_test_contract(api).await
                                                            }

                                                            None => Err(format!(
                                                                "Freenet connection closed before {action_name} verification."
                                                            )),
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
                    move |error| {
                        contract_for_host_error
                            .set(crate::transport::host_result_error_status(&error));
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

        let pending_role_check = load_pending_action(TEST_CONTRACT_ID);

        let no_pending_action = matches!(&pending_role_check, Ok(None));

        /*
         * Only the browser profile holding the active player role may prepare
         * checker movement. The protocol still independently validates the
         * completed sequence before any network submission.
         */
        let local_role_has_turn =
            session_active && authoritative_player_role == Some(controller.state().active_player);

        let controls_authoritative_turn = local_role_has_turn && no_pending_action;

        /*
         * Dice are produced by the commit-and-reveal action loop. The former
         * local random Roll control must not alter a network game.
         */
        let authoritative_roll_ready = latest_authoritative_state
            .borrow()
            .as_ref()
            .and_then(|state_bytes| decode_verified_replay(state_bytes).ok())
            .map(|replay| {
                authoritative_player_role == Some(replay.state.active_player)
                    && replay.state.turn_phase == TurnPhase::AwaitingRoll
                    && replay.state.dice.is_none()
                    && replay.roll_requested_by.is_none()
                    && replay.dice_round.is_empty()
            })
            .unwrap_or(false);

        let can_roll = session_active
            && no_pending_action
            && latest_contract_key.borrow().is_some()
            && authoritative_roll_ready;

        let authoritative_must_pass = session_active && controller.must_pass();
        let can_pass = controls_authoritative_turn && authoritative_must_pass;

        /*
         * Resignation is not networked yet. Disable the local-only mutation so
         * it cannot falsely present a terminal network result.
         */
        let can_resign = false;

        let active_name = player_name(board.active_player);

        let white_player_name = match authoritative_player_role {
            Some(Player::White) => "Player One (You)",
            Some(Player::Black) => "Player One (Opponent)",
            None => "Player One",
        }
        .to_owned();

        let black_player_name = match authoritative_player_role {
            Some(Player::Black) => "Player Two (You)",
            Some(Player::White) => "Player Two (Opponent)",
            None => "Player Two",
        }
        .to_owned();

        let turn_text = if left_table {
            "Table left".to_owned()
        } else if outcome.is_some() {
            "Game complete".to_owned()
        } else if authoritative_must_pass {
            format!("{active_name} must pass")
        } else {
            match board.turn_phase {
                TurnPhase::AwaitingRoll => {
                    format!("{active_name} awaiting fair roll")
                }
                TurnPhase::Moving => {
                    format!("{active_name} is moving")
                }
            }
        };

        let game_state_text = if left_table {
            "Table left".to_owned()
        } else if outcome.is_some() {
            "Game complete".to_owned()
        } else if authoritative_must_pass {
            format!("Awaiting {active_name} pass")
        } else {
            match board.turn_phase {
                TurnPhase::AwaitingRoll => format!("Awaiting {active_name} roll"),
                TurnPhase::Moving => format!("{active_name} moving"),
            }
        };

        let control_note = if left_table {
            "The local table session has ended.".to_owned()
        } else if outcome.is_some() {
            "Begin a new game to play again.".to_owned()
        } else if authoritative_must_pass {
            if can_pass {
                "The roll has no legal move. Pass the turn when ready.".to_owned()
            } else if local_role_has_turn && !no_pending_action {
                "Waiting for Freenet to confirm the pending action.".to_owned()
            } else {
                format!("Waiting for {active_name} to pass the turn.")
            }
        } else {
            match board.turn_phase {
                TurnPhase::AwaitingRoll => {
                    if can_roll {
                        "Roll to begin the turn.".to_owned()
                    } else if local_role_has_turn && !no_pending_action {
                        "Waiting for Freenet to confirm the pending action.".to_owned()
                    } else {
                        format!("Waiting for {active_name} to roll.")
                    }
                }
                TurnPhase::Moving => {
                    if local_role_has_turn {
                        if no_pending_action {
                            "Complete the turn when ready.".to_owned()
                        } else {
                            "Waiting for Freenet to confirm the pending action.".to_owned()
                        }
                    } else {
                        format!("Waiting for {active_name} to complete the turn.")
                    }
                }
            }
        };

        let legal_sources =
            if controls_authoritative_turn && controller.state().turn_phase == TurnPhase::Moving {
                controller.legal_sources()
            } else {
                Vec::new()
            };

        let selected_source = if controls_authoritative_turn {
            controller.selected_source()
        } else {
            None
        };

        let legal_destinations = if controls_authoritative_turn && selected_source.is_some() {
            controller.legal_destinations().unwrap_or_default()
        } else {
            Vec::new()
        };

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

        let submit_pending_secretless_action = {
            let freenet_api = freenet_api.clone();
            let contract_status = contract_status.clone();
            let interface_error = interface_error.clone();
            let local_network_action_submitted = local_network_action_submitted.clone();

            Callback::from(
                move |(key, pending): (
                    freenet_stdlib::prelude::ContractKey,
                    crate::pending_action::PendingAction,
                )| {
                    let action_id = pending.action_id;
                    let delta = pending.delta;

                    {
                        let mut submitted = local_network_action_submitted.borrow_mut();

                        if submitted.as_ref() == Some(&action_id) {
                            return;
                        }

                        *submitted = Some(action_id);
                    }

                    interface_error.set(None);
                    contract_status.set(ContractProbeStatus::Updating);

                    let api_for_update = freenet_api.clone();
                    let contract_for_update = contract_status.clone();
                    let interface_for_update = interface_error.clone();

                    wasm_bindgen_futures::spawn_local(async move {
                        let submit_result = {
                            let mut api = api_for_update.borrow_mut();

                            match api.as_mut() {
                                Some(api) => submit_action_delta(api, key, delta).await,

                                None => Err(
                                    "Freenet connection closed before the pending action update."
                                        .to_owned(),
                                ),
                            }
                        };

                        match submit_result {
                            Ok(()) => {
                                contract_for_update.set(ContractProbeStatus::VerifyingUpdate);

                                gloo_timers::future::TimeoutFuture::new(750).await;

                                let refresh_result = {
                                    let mut api = api_for_update.borrow_mut();

                                    match api.as_mut() {
                                        Some(api) => request_test_contract(api).await,

                                        None => Err(
                                            "Freenet connection closed before pending action verification."
                                                .to_owned(),
                                        ),
                                    }
                                };

                                if let Err(error) = refresh_result {
                                    interface_for_update.set(Some(format!(
                                        "The submitted action could not be refreshed: {error}"
                                    )));

                                    contract_for_update.set(ContractProbeStatus::Failed(error));
                                }
                            }

                            Err(error) => {
                                /*
                                 * The durable pending action remains intact.
                                 * Reconnect retries the same action ID and
                                 * exact encoded delta.
                                 */
                                interface_for_update
                                    .set(Some(format!("The action submission failed: {error}")));

                                contract_for_update.set(ContractProbeStatus::Failed(error));
                            }
                        }
                    });
                },
            )
        };

        let on_roll = {
            let interface_error = interface_error.clone();
            let latest_contract_key = latest_contract_key.clone();
            let latest_authoritative_state = latest_authoritative_state.clone();
            let submit_pending_secretless_action = submit_pending_secretless_action.clone();
            let authoritative_player_role = authoritative_player_role;

            Callback::from(move |_| {
                let prepared = (|| -> Result<
                    (
                        freenet_stdlib::prelude::ContractKey,
                        crate::pending_action::PendingAction,
                    ),
                    String,
                > {
                    let local_player = authoritative_player_role
                        .ok_or_else(|| "This browser identity does not have an authoritative player role.".to_owned())?;

                    let key = latest_contract_key
                        .borrow()
                        .clone()
                        .ok_or_else(|| "No verified Freenet contract key is available.".to_owned())?;

                    let state_bytes = latest_authoritative_state
                        .borrow()
                        .clone()
                        .ok_or_else(|| "No verified authoritative parent state is available.".to_owned())?;

                    match plan_browser_request_roll(&state_bytes, local_player, true)? {
                        RequestRollPlan::Submit {
                            pending,
                            recovered_pending: false,
                        } => Ok((key, pending)),

                        RequestRollPlan::Submit {
                            recovered_pending: true,
                            ..
                        } => Err(
                            "A prior pending roll request already exists; reconnect to recover it."
                                .to_owned(),
                        ),

                        RequestRollPlan::NoAction => {
                            Err("The roll request did not produce a network action.".to_owned())
                        }

                        RequestRollPlan::Accepted => Err(
                            "The roll request was already accepted before this submission."
                                .to_owned(),
                        ),
                    }
                })();

                match prepared {
                    Ok((key, pending)) => {
                        interface_error.set(None);
                        submit_pending_secretless_action.emit((key, pending));
                    }

                    Err(error) => {
                        interface_error.set(Some(error));
                    }
                }
            })
        };

        let on_pass = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();
            let latest_contract_key = latest_contract_key.clone();
            let latest_authoritative_state = latest_authoritative_state.clone();
            let submit_pending_secretless_action = submit_pending_secretless_action.clone();
            let authoritative_player_role = authoritative_player_role;

            Callback::from(move |_| {
                let mut next = (*controller).clone();

                let prepared = (|| -> Result<
                    (
                        freenet_stdlib::prelude::ContractKey,
                        crate::pending_action::PendingAction,
                    ),
                    String,
                > {
                    let sequence = next
                        .prepare_pass_for_submission()
                        .map_err(|error| {
                            format!(
                                "The forced pass could not be prepared: {error:?}"
                            )
                        })?;

                    let local_player =
                        authoritative_player_role.ok_or_else(|| {
                            "This browser identity does not have an authoritative player role."
                                .to_owned()
                        })?;

                    let key = latest_contract_key
                        .borrow()
                        .clone()
                        .ok_or_else(|| {
                            "No verified Freenet contract key is available."
                                .to_owned()
                        })?;

                    let state_bytes = latest_authoritative_state
                        .borrow()
                        .clone()
                        .ok_or_else(|| {
                            "No verified authoritative parent state is available."
                                .to_owned()
                        })?;

                    match plan_browser_play_turn(
                        &state_bytes,
                        local_player,
                        Some(&sequence),
                    )? {
                        PlayTurnPlan::Submit {
                            pending,
                            recovered_pending: false,
                        } => Ok((key, pending)),

                        PlayTurnPlan::Submit {
                            recovered_pending: true,
                            ..
                        } => Err(
                            "A prior pending turn already exists; reconnect to recover it."
                                .to_owned(),
                        ),

                        PlayTurnPlan::NoAction => Err(
                            "The prepared pass did not produce a network action."
                                .to_owned(),
                        ),

                        PlayTurnPlan::Accepted => Err(
                            "The pass was already accepted before this submission."
                                .to_owned(),
                        ),
                    }
                })();

                match prepared {
                    Ok((key, pending)) => {
                        interface_error.set(None);
                        controller.set(next);
                        submit_pending_secretless_action.emit((key, pending));
                    }

                    Err(error) => {
                        interface_error.set(Some(error));
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
            let latest_contract_key = latest_contract_key.clone();
            let latest_authoritative_state = latest_authoritative_state.clone();
            let submit_pending_secretless_action = submit_pending_secretless_action.clone();
            let authoritative_player_role = authoritative_player_role;

            Callback::from(move |destination: MoveTarget| {
                let mut next = (*controller).clone();

                match next.choose_destination_for_submission(destination) {
                    Ok(None) => {
                        /*
                         * A partial legal sequence changes only the transient
                         * preview. No network action exists yet.
                         */
                        interface_error.set(None);
                        controller.set(next);
                    }

                    Ok(Some(sequence)) => {
                        let prepared = (|| -> Result<
                            (
                                freenet_stdlib::prelude::ContractKey,
                                crate::pending_action::PendingAction,
                            ),
                            String,
                        > {
                            let local_player =
                                authoritative_player_role.ok_or_else(|| {
                                    "This browser identity does not have an authoritative player role."
                                        .to_owned()
                                })?;

                            let key = latest_contract_key
                                .borrow()
                                .clone()
                                .ok_or_else(|| {
                                    "No verified Freenet contract key is available."
                                        .to_owned()
                                })?;

                            let state_bytes =
                                latest_authoritative_state
                                    .borrow()
                                    .clone()
                                    .ok_or_else(|| {
                                        "No verified authoritative parent state is available."
                                            .to_owned()
                                    })?;

                            match plan_browser_play_turn(
                                &state_bytes,
                                local_player,
                                Some(&sequence),
                            )? {
                                PlayTurnPlan::Submit {
                                    pending,
                                    recovered_pending: false,
                                } => Ok((key, pending)),

                                PlayTurnPlan::Submit {
                                    recovered_pending: true,
                                    ..
                                } => Err(
                                    "A prior pending turn already exists; reconnect to recover it."
                                        .to_owned(),
                                ),

                                PlayTurnPlan::NoAction => Err(
                                    "The completed checker sequence did not produce a network action."
                                        .to_owned(),
                                ),

                                PlayTurnPlan::Accepted => Err(
                                    "The completed turn was already accepted before submission."
                                        .to_owned(),
                                ),
                            }
                        })();

                        match prepared {
                            Ok((key, pending)) => {
                                /*
                                 * Keep the completed checker arrangement only
                                 * as a preview. Authoritative acceptance will
                                 * replace committed and preview state together.
                                 */
                                interface_error.set(None);
                                controller.set(next);
                                submit_pending_secretless_action.emit((key, pending));
                            }

                            Err(error) => {
                                /*
                                 * Do not retain the final preview move when no
                                 * durable canonical action was created.
                                 */
                                interface_error.set(Some(error));
                            }
                        }
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
            let connection_status = connection_status.clone();
            let contract_status = contract_status.clone();
            let subscription_status = subscription_status.clone();
            let freenet_api = freenet_api.clone();
            let local_network_action_submitted = local_network_action_submitted.clone();
            let local_dice_secret = local_dice_secret.clone();
            let dice_secret_status = dice_secret_status.clone();
            let latest_contract_key = latest_contract_key.clone();
            let latest_authoritative_state = latest_authoritative_state.clone();
            let local_player_id_for_reconnect = local_player_id.clone();
            let authoritative_local_role_for_reconnect = authoritative_local_role.clone();
            let controller_for_reconnect = controller.clone();

            Callback::from(move |_| {
                freenet_api.borrow_mut().take();

                /*
                 * Permit one submission attempt on the new connection.
                 * The durable pending action itself remains unchanged.
                 */
                *local_network_action_submitted.borrow_mut() = None;
                latest_contract_key.borrow_mut().take();
                latest_authoritative_state.borrow_mut().take();
                authoritative_local_role_for_reconnect.set(None);

                contract_status.set(ContractProbeStatus::WaitingForConnection);
                subscription_status.set(SubscriptionStatus::Pending);
                dice_secret_status.set("Checking browser storage".to_owned());

                let status_for_callback = connection_status.clone();
                let contract_for_response = contract_status.clone();
                let contract_for_host_error = contract_status.clone();
                let subscription_for_response = subscription_status.clone();
                let subscription_for_status = subscription_status.clone();
                let api_for_response = freenet_api.clone();
                let network_action_for_response = local_network_action_submitted.clone();
                let secret_for_response = local_dice_secret.clone();
                let secret_status_for_response = dice_secret_status.clone();
                let key_for_response = latest_contract_key.clone();
                let state_for_response = latest_authoritative_state.clone();
                let player_id_for_response = local_player_id_for_reconnect.clone();
                let authoritative_role_for_response =
                    authoritative_local_role_for_reconnect.clone();
                let controller_for_response = controller_for_reconnect.clone();

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
                            } = classified;

                            if let Some(contract) = contract_status {
                                contract_for_response.set(contract);
                            }

                            if let Some(subscription) = subscription_status {
                                subscription_for_response.set(subscription);
                            }

                            if let (Some(key), Some(state_bytes)) =
                                (contract_key, authoritative_state)
                            {
                                let authoritative_state =
                                    match authoritative_game_projection(&state_bytes) {
                                        Ok(state) => state,

                                        Err(error) => {
                                            contract_for_response.set(ContractProbeStatus::Failed(
                                                format!("Authoritative replay failed: {error}"),
                                            ));

                                            return;
                                        }
                                    };

                                /*
                                 * An unchanged parent ledger must not erase a
                                 * local checker preview while a turn is being
                                 * prepared or awaiting authoritative acceptance.
                                 */
                                let state_changed =
                                    state_for_response.borrow().as_ref() != Some(&state_bytes);

                                /*
                                 * Cache only after complete ledger decoding and
                                 * deterministic protocol replay have succeeded.
                                 */
                                *key_for_response.borrow_mut() = Some(key.clone());
                                *state_for_response.borrow_mut() = Some(state_bytes.clone());

                                let resolved_role = match *player_id_for_response {
                                    None => None,
                                    Some(player_id) => {
                                        decode_verified_replay(&state_bytes).ok().map(|replay| {
                                            role_for_player_id(&replay.configuration, &player_id)
                                        })
                                    }
                                };

                                let authoritative_player = resolved_role.flatten();
                                authoritative_role_for_response.set(resolved_role);

                                if state_changed {
                                    if let Some((authoritative_state, authoritative_history)) =
                                        authoritative_state
                                    {
                                        let mut next = (*controller_for_response).clone();

                                        if let Err(error) = next
                                            .sync_authoritative_state_and_history(
                                                authoritative_state,
                                                authoritative_history,
                                            )
                                        {
                                            contract_for_response.set(
                                                ContractProbeStatus::Failed(
                                                    format!(
                                                        "Authoritative board synchronization failed: {error:?}"
                                                    ),
                                                ),
                                            );

                                            return;
                                        }

                                        controller_for_response.set(next);
                                    }
                                }

                                let Some(local_player) = authoritative_player else {
                                    secret_status_for_response
                                        .set("This browser identity is not an authoritative game participant".to_owned());
                                    return;
                                };

                                let plan =
                                    match plan_browser_network_action(&state_bytes, local_player) {
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
                                    BrowserNetworkActionPlan::NoAction => {
                                        secret_status_for_response
                                            .set("No dice-secret recovery needed".to_owned());
                                    }

                                    BrowserNetworkActionPlan::Accepted { secret, kind } => {
                                        *secret_for_response.borrow_mut() = Some(secret);
                                        *network_action_for_response.borrow_mut() = None;

                                        secret_status_for_response.set(match kind {
                                            NetworkActionKind::Commitment => {
                                                "Recovered and matched accepted commitment"
                                                    .to_owned()
                                            }

                                            NetworkActionKind::Reveal => {
                                                "Recovered and matched accepted reveal".to_owned()
                                            }
                                        });
                                    }

                                    BrowserNetworkActionPlan::SecretlessAccepted { kind: _ } => {
                                        /*
                                         * The authoritative board was already
                                         * synchronized above from the accepted
                                         * history. Only the local submission
                                         * guard remains to be cleared.
                                         */
                                        *network_action_for_response.borrow_mut() = None;
                                    }

                                    BrowserNetworkActionPlan::SecretlessSubmit {
                                        pending,
                                        recovered_pending,
                                        kind: _,
                                    } => {
                                        let action_id = pending.action_id;
                                        let delta = pending.delta;

                                        {
                                            let mut submitted =
                                                network_action_for_response.borrow_mut();

                                            if submitted.as_ref() == Some(&action_id) {
                                                return;
                                            }

                                            *submitted = Some(action_id);
                                        }

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
                                                            "Freenet connection closed before the pending turn update."
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
                                                                    "Freenet connection closed before turn verification."
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
                                                     * Keep the exact pending
                                                     * turn. Reconnect will
                                                     * retry the same bytes.
                                                     */
                                                    contract_for_update.set(
                                                        ContractProbeStatus::Failed(format!(
                                                            "{}{}",
                                                            if recovered_pending {
                                                                "Recovered turn retry failed: "
                                                            } else {
                                                                "Turn submission failed: "
                                                            },
                                                            error,
                                                        )),
                                                    );
                                                }
                                            }
                                        });
                                    }

                                    BrowserNetworkActionPlan::Submit {
                                        secret,
                                        pending,
                                        recovered_pending,
                                        kind,
                                    } => {
                                        let action_id = pending.action_id;
                                        let delta = pending.delta;

                                        {
                                            let mut submitted =
                                                network_action_for_response.borrow_mut();

                                            if submitted.as_ref() == Some(&action_id) {
                                                return;
                                            }

                                            *submitted = Some(action_id);
                                        }

                                        *secret_for_response.borrow_mut() = Some(secret);

                                        let action_name = match kind {
                                            NetworkActionKind::Commitment => "commitment",
                                            NetworkActionKind::Reveal => "reveal",
                                        };

                                        secret_status_for_response.set(if recovered_pending {
                                            format!(
                                                "Recovered exact pending {action_name}; retrying"
                                            )
                                        } else {
                                            format!(
                                                "Stored {action_name} locally; awaiting network verification"
                                            )
                                        });

                                        contract_for_response.set(ContractProbeStatus::Updating);

                                        let api_for_update = api_for_response.clone();
                                        let contract_for_update = contract_for_response.clone();

                                        wasm_bindgen_futures::spawn_local(async move {
                                            let submit_result = {
                                                let mut api = api_for_update.borrow_mut();

                                                match api.as_mut() {
                                                    Some(api) => {
                                                        submit_action_delta(api, key, delta).await
                                                    }

                                                    None => Err(format!(
                                                        "Freenet connection closed before the pending {action_name} update."
                                                    )),
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
                                                                request_test_contract(api).await
                                                            }

                                                            None => Err(format!(
                                                                "Freenet connection closed before {action_name} verification."
                                                            )),
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
                                                     * Preserve the exact durable pending record.
                                                     * A reconnect retries the same action ID and
                                                     * exact encoded delta.
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
                    move |error| {
                        contract_for_host_error
                            .set(crate::transport::host_result_error_status(&error));
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
                            name={black_player_name}
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
                            name={white_player_name}
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
                            status_note={control_note}
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
                                <dt>{ "Identity role" }</dt>
                                <dd>{ authoritative_identity_role_text }</dd>
                            </div>


                                <div>
                                    <dt>{ "Player ID" }</dt>
                                    <dd>{ local_player_id_text }</dd>
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
                                    <dd>{ game_state_text }</dd>
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
