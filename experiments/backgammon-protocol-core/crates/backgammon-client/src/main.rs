#[cfg(target_arch = "wasm32")]
mod components;
mod game_contract_publication;

pub mod accepted_game_projection;
pub mod challenge;
pub mod challenge_offer_planner;
pub mod challenge_publication_store;
pub mod challenge_state;
pub mod commitment_planner;
pub mod controller;
pub mod genesis_handshake;
pub mod genesis_handshake_store;
pub mod incoming_challenge_acceptance_planner;
pub mod incoming_challenge_acceptance_store;
pub mod incoming_challenge_acceptance_transport;
pub mod incoming_challenge_projection;
pub mod ledger_codec;
pub mod lobby;
pub mod lobby_codec;
pub mod lobby_presence_planner;
pub mod lobby_profile_store;
pub mod lobby_projection;
pub mod lobby_transport;
pub mod local_identity_store;
pub mod local_role_store;
pub mod pending_action;
pub mod pending_action_store;
pub mod play_turn_planner;
pub mod presence_revision_store;
pub mod projection;
pub mod request_roll_planner;
pub mod reveal_planner;
pub mod secret_store;
pub mod transport;

#[cfg(test)]
mod test_support;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::{cell::RefCell, rc::Rc};

    use backgammon_core::{GameState, MoveSource, MoveTarget, Player, TurnPhase, TurnSequence};
    use backgammon_lobby_core::LobbyContractState;
    use backgammon_protocol::{
        replay_game, verify_challenge_offer_at, DiceSecret, GameActionPayload, SignedChallengeOffer,
    };
    use freenet_stdlib::client_api::{HostResponse, WebApi};
    use freenet_stdlib::prelude::ContractKey;
    use js_sys::Date;
    use yew::prelude::*;
    use yew::TargetCast;

    use crate::accepted_game_projection::project_accepted_games;
    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};
    use crate::challenge_publication_store::{
        load_outbound_challenge_publication, remove_outbound_challenge_publication,
        store_new_outbound_challenge_publication, update_outbound_challenge_publication,
        OutboundChallengePublicationStage, StoredOutboundChallengePublication,
    };
    use crate::commitment_planner::{plan_commitment, CommitmentPlan, CommitmentPlannerInput};
    use crate::components::board::Board;
    use crate::components::controls::GameControls;
    use crate::components::dice::DiceDisplay;
    use crate::components::history::MoveHistory;
    use crate::components::player_panel::PlayerPanel;
    use crate::controller::{LocalGameController, LocalGameOutcome, LocalTurnRecord};
    use crate::game_contract_publication::{
        confirm_game_contract_publication, submit_game_contract_publication,
        SubmittedGameContractPublication,
    };
    use crate::incoming_challenge_acceptance_planner::{
        finalize_incoming_challenge_acceptance, prepare_incoming_challenge_contract_probe,
        IncomingChallengeContractProbe,
    };
    use crate::incoming_challenge_acceptance_store::{
        load_incoming_challenge_acceptance, remove_incoming_challenge_acceptance,
        store_new_incoming_challenge_acceptance, StoredIncomingChallengeAcceptance,
    };
    use crate::incoming_challenge_acceptance_transport::{
        classify_incoming_challenge_contract_response, IncomingChallengeContractRead,
    };
    use crate::incoming_challenge_projection::project_incoming_challenges;
    use crate::ledger_codec::{decode_verified_ledger, decode_verified_replay};
    use crate::lobby_presence_planner::{plan_lobby_presence, LobbyPresencePlannerInput};
    use crate::lobby_profile_store::{load_lobby_display_name, store_lobby_display_name};
    use crate::lobby_projection::project_available_players;
    use crate::lobby_transport::{
        classify_lobby_response, request_lobby_contract, submit_lobby_state_update,
        ClassifiedLobbyResponse, LobbyContractStatus,
    };
    use crate::local_identity_store::{
        load_local_identity, load_or_create_local_identity, player_id_for_signing_key,
        role_for_player_id,
    };
    use crate::local_role_store::{load_local_role, store_local_role};
    use crate::pending_action_store::{
        load_pending_action, remove_pending_action, store_pending_action,
    };
    use crate::play_turn_planner::{plan_play_turn, PlayTurnPlan, PlayTurnPlannerInput};
    use crate::presence_revision_store::reserve_next_presence_revision;
    use crate::projection::BoardView;
    use crate::request_roll_planner::{
        plan_request_roll, RequestRollPlan, RequestRollPlannerInput,
    };
    use crate::reveal_planner::{plan_reveal, RevealPlan, RevealPlannerInput};
    use crate::secret_store::{load_dice_secret, store_dice_secret};
    use crate::transport::{
        classify_response, connect, request_contract, submit_action_delta, ClassifiedResponse,
        ConnectionStatus, ContractProbeStatus, SubscriptionStatus, TEST_CONTRACT_ID,
    };

    fn format_player_id(player_id: &[u8; 32]) -> String {
        player_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join("")
    }

    /*
     * Local wall-clock time is advisory discovery input only. It may filter
     * this browser's view of expired presence records, but it must never decide
     * authoritative lobby state, game state, action ordering, or abandonment.
     */
    fn local_observation_unix_seconds() -> Result<u64, String> {
        let milliseconds = Date::now();

        if !milliseconds.is_finite() || milliseconds < 0.0 {
            return Err("Browser Date.now() returned an invalid value.".to_owned());
        }

        let seconds = (milliseconds / 1_000.0).floor();

        if seconds > u64::MAX as f64 {
            return Err("Browser Date.now() exceeds the supported Unix range.".to_owned());
        }

        Ok(seconds as u64)
    }

    /// Removes durable outbound publication state only after the exact signed
    /// offer appears in a complete, independently verified lobby state.
    fn observe_authoritative_challenge_publication(
        state: &LobbyContractState,
        local_player_id_handle: &UseStateHandle<Option<[u8; 32]>>,
        pending_handle: &UseStateHandle<bool>,
        status_handle: &UseStateHandle<String>,
        error_handle: &UseStateHandle<Option<String>>,
    ) {
        let Some(local_player_id) = **local_player_id_handle else {
            return;
        };

        let observation = (|| {
            let Some(stored) = load_outbound_challenge_publication(&local_player_id)? else {
                return Ok(false);
            };

            if stored.stage != OutboundChallengePublicationStage::AwaitingLobbyConfirmation {
                return Ok(false);
            }

            if !stored.is_exact_offer_authoritative(state)? {
                return Ok(false);
            }

            remove_outbound_challenge_publication(&local_player_id, &stored.challenge_id())?;

            Ok(true)
        })();

        match observation {
            Ok(true) => {
                pending_handle.set(false);
                status_handle.set(
                    "Challenge advertised and confirmed in verified authoritative lobby state"
                        .to_owned(),
                );
                error_handle.set(None);
            }

            Ok(false) => {}

            Err(error) => {
                /*
                 * Retain the pending state and surface the error. Storage removal
                 * itself verifies that the exact identity-scoped record vanished.
                 */
                pending_handle.set(true);
                status_handle
                    .set("Challenge publication confirmation requires recovery".to_owned());
                error_handle.set(Some(error));
            }
        }
    }

    /// Removes durable incoming acceptance evidence only after the exact
    /// signed offer and exact signed acceptance appear together in a complete,
    /// independently verified authoritative lobby state.
    fn observe_authoritative_incoming_challenge_acceptance(
        state: &LobbyContractState,
        local_player_id_handle: &UseStateHandle<Option<[u8; 32]>>,
        pending_handle: &UseStateHandle<bool>,
        status_handle: &UseStateHandle<String>,
        error_handle: &UseStateHandle<Option<String>>,
    ) {
        let Some(local_player_id) = **local_player_id_handle else {
            return;
        };

        let observation = (|| {
            let Some(stored) = load_incoming_challenge_acceptance(&local_player_id)? else {
                return Ok(false);
            };

            if !stored.is_exact_acceptance_authoritative(state)? {
                return Ok(false);
            }

            remove_incoming_challenge_acceptance(&local_player_id, &stored.challenge_id())?;

            Ok(true)
        })();

        match observation {
            Ok(true) => {
                pending_handle.set(false);
                status_handle.set(
                    "Challenge acceptance confirmed in verified authoritative \
                     lobby state"
                        .to_owned(),
                );
                error_handle.set(None);
            }

            Ok(false) => {}

            Err(error) => {
                /*
                 * Retain the durable record and pending state. Removal itself
                 * verifies that the exact identity-scoped record vanished.
                 */
                pending_handle.set(true);
                status_handle.set("Challenge acceptance confirmation requires recovery".to_owned());
                error_handle.set(Some(error));
            }
        }
    }

    fn handle_lobby_response(
        response: &HostResponse,
        contract_status_handle: &UseStateHandle<LobbyContractStatus>,
        subscription_status_handle: &UseStateHandle<SubscriptionStatus>,
        authoritative_state_handle: &UseStateHandle<Option<LobbyContractState>>,
        contract_key_handle: &Rc<RefCell<Option<ContractKey>>>,
        api_handle: &Rc<RefCell<Option<WebApi>>>,
        local_player_id_handle: &UseStateHandle<Option<[u8; 32]>>,
        challenge_pending_handle: &UseStateHandle<bool>,
        challenge_status_handle: &UseStateHandle<String>,
        challenge_error_handle: &UseStateHandle<Option<String>>,
        incoming_pending_handle: &UseStateHandle<bool>,
        incoming_status_handle: &UseStateHandle<String>,
        incoming_error_handle: &UseStateHandle<Option<String>>,
    ) -> bool {
        let Some(classified) = classify_lobby_response(response) else {
            return false;
        };

        let ClassifiedLobbyResponse {
            contract_status,
            subscription_status,
            contract_key,
            authoritative_state,
            refresh_required,
        } = classified;

        if let Some(status) = contract_status {
            contract_status_handle.set(status);
        }

        if let Some(status) = subscription_status {
            subscription_status_handle.set(status);
        }

        if let Some(key) = contract_key {
            *contract_key_handle.borrow_mut() = Some(key);
        }

        if let Some(state) = authoritative_state {
            observe_authoritative_challenge_publication(
                &state,
                local_player_id_handle,
                challenge_pending_handle,
                challenge_status_handle,
                challenge_error_handle,
            );

            observe_authoritative_incoming_challenge_acceptance(
                &state,
                local_player_id_handle,
                incoming_pending_handle,
                incoming_status_handle,
                incoming_error_handle,
            );

            authoritative_state_handle.set(Some(state));
        }

        if refresh_required {
            let api = api_handle.clone();
            let contract_status = contract_status_handle.clone();
            let subscription_status = subscription_status_handle.clone();

            wasm_bindgen_futures::spawn_local(async move {
                let result = {
                    let mut api = api.borrow_mut();

                    match api.as_mut() {
                        Some(api) => request_lobby_contract(api).await,
                        None => {
                            Err("Freenet connection closed before the lobby refresh.".to_owned())
                        }
                    }
                };

                if let Err(error) = result {
                    contract_status.set(LobbyContractStatus::Failed(error));
                    subscription_status.set(SubscriptionStatus::Inactive);
                }
            });
        }

        true
    }

    /// Finalizes only the direct contract read for the currently armed
    /// incoming-challenge probe.
    ///
    /// Every non-signing prerequisite is resolved before finalization begins.
    /// The resulting signature is converted to read-back-verified durable
    /// evidence before any lobby update is submitted.
    fn handle_incoming_challenge_contract_response(
        response: &HostResponse,
        armed_probe_handle: &Rc<RefCell<Option<IncomingChallengeContractProbe>>>,
        retrieved_read_handle: &Rc<
            RefCell<Option<(IncomingChallengeContractProbe, ContractKey, Vec<u8>)>>,
        >,
        authoritative_state_handle: &UseStateHandle<Option<LobbyContractState>>,
        local_player_id_handle: &UseStateHandle<Option<[u8; 32]>>,
        lobby_key_handle: &Rc<RefCell<Option<ContractKey>>>,
        api_handle: &Rc<RefCell<Option<WebApi>>>,
        pending_handle: &UseStateHandle<bool>,
        status_handle: &UseStateHandle<String>,
        error_handle: &UseStateHandle<Option<String>>,
    ) -> bool {
        let Some(probe) = armed_probe_handle.borrow().clone() else {
            return false;
        };

        let Some(classified) =
            classify_incoming_challenge_contract_response(response, &probe.contract_id)
        else {
            return false;
        };

        armed_probe_handle.borrow_mut().take();

        let IncomingChallengeContractRead::Retrieved {
            contract_key,
            state,
        } = classified
        else {
            retrieved_read_handle.borrow_mut().take();
            pending_handle.set(false);
            status_handle.set("Game contract was not found; no acceptance was created".to_owned());
            error_handle.set(Some(format!(
                "The challenged game contract {} is unavailable.",
                probe.contract_id,
            )));
            return true;
        };

        /*
         * Keep the response and its originating probe inseparable until this
         * handler consumes them for finalization.
         */
        *retrieved_read_handle.borrow_mut() = Some((probe, contract_key, state));

        let Some((probe, contract_key, state)) = retrieved_read_handle.borrow_mut().take() else {
            pending_handle.set(false);
            status_handle.set("Incoming contract response could not be retained".to_owned());
            error_handle.set(Some(
                "The direct contract response lost its originating probe.".to_owned(),
            ));
            return true;
        };

        let mut finalization_started = false;

        let prepared = (|| {
            let active_player_id = (**local_player_id_handle).ok_or_else(|| {
                "Local identity disappeared before acceptance finalization.".to_owned()
            })?;

            if active_player_id != probe.local_player_id {
                return Err("Active PlayerId differs from the challenged recipient.".to_owned());
            }

            if load_incoming_challenge_acceptance(&active_player_id)?.is_some() {
                return Err("A durable incoming challenge acceptance already exists \
                     for this identity."
                    .to_owned());
            }

            let authoritative_state = (**authoritative_state_handle).as_ref().ok_or_else(|| {
                "Authoritative lobby state disappeared before \
                         acceptance finalization."
                    .to_owned()
            })?;

            let mut exact_matches = authoritative_state
                .challenges
                .offers
                .iter()
                .filter(|entry| entry.offer == probe.signed_offer);

            let current_challenge = exact_matches.next().ok_or_else(|| {
                "The probed signed challenge is no longer present in \
                     authoritative lobby state."
                    .to_owned()
            })?;

            if exact_matches.next().is_some() {
                return Err("Authoritative lobby state contains ambiguous duplicate \
                     records for the probed challenge."
                    .to_owned());
            }

            let signing_key = load_local_identity()?
                .ok_or_else(|| "Persistent signing identity is unavailable.".to_owned())?;

            if player_id_for_signing_key(&signing_key) != active_player_id {
                return Err("Persistent signing identity does not match the active \
                     PlayerId."
                    .to_owned());
            }

            let now_unix_seconds = local_observation_unix_seconds()?;

            let lobby_key = lobby_key_handle
                .borrow()
                .clone()
                .ok_or_else(|| "The verified lobby contract key is unavailable.".to_owned())?;

            if api_handle.borrow().is_none() {
                return Err("The Freenet connection closed before acceptance \
                     finalization."
                    .to_owned());
            }

            /*
             * From this point onward a signature may be created. Any error is
             * therefore treated conservatively as requiring recovery.
             */
            finalization_started = true;

            let plan = finalize_incoming_challenge_acceptance(
                &probe,
                current_challenge,
                &contract_key,
                &state,
                &signing_key,
                now_unix_seconds,
            )?;

            let stored = StoredIncomingChallengeAcceptance::new(&plan)?;

            let challenge_id = stored.challenge_id();

            /*
             * This exact read-back-verified durable write must complete before
             * the signed lobby update is allowed onto the network.
             */
            store_new_incoming_challenge_acceptance(&stored)?;

            Ok((lobby_key, plan.encoded_lobby_state_update, challenge_id))
        })();

        let (lobby_key, encoded_lobby_state_update, challenge_id) = match prepared {
            Ok(prepared) => prepared,

            Err(error) => {
                pending_handle.set(finalization_started);

                if finalization_started {
                    status_handle
                        .set("Acceptance finalization requires durable recovery".to_owned());
                } else {
                    status_handle
                        .set("Contract proof was rejected; no acceptance was created".to_owned());
                }

                error_handle.set(Some(error));
                return true;
            }
        };

        pending_handle.set(true);
        error_handle.set(None);
        status_handle
            .set("Acceptance verified and stored; publishing its exact lobby update".to_owned());

        let api = api_handle.clone();
        let pending = pending_handle.clone();
        let status = status_handle.clone();
        let error = error_handle.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = {
                let mut api = api.borrow_mut();

                match api.as_mut() {
                    Some(api) => {
                        submit_lobby_state_update(api, lobby_key, encoded_lobby_state_update).await
                    }

                    None => Err("Freenet connection closed before the stored \
                         acceptance could be published."
                        .to_owned()),
                }
            };

            let short_challenge_id = challenge_id[..5]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();

            match result {
                Ok(()) => {
                    pending.set(true);
                    status.set(format!(
                        "Acceptance for challenge {short_challenge_id}… \
                         submitted; awaiting verified authoritative confirmation",
                    ));
                    error.set(None);
                }

                Err(submission_error) => {
                    /*
                     * Durable signed evidence remains. Recovery must rebuild and
                     * resend those exact bytes without signing again.
                     */
                    pending.set(true);
                    status.set("Acceptance is stored; lobby publication requires retry".to_owned());
                    error.set(Some(submission_error));
                }
            }
        });

        true
    }

    /// Routes only the `PutResponse` for the currently armed per-game
    /// contract publication.
    ///
    /// Exact contract confirmation is persisted before the signed challenge is
    /// submitted to the lobby. The durable record remains until that exact offer
    /// is later observed in independently verified authoritative lobby state.
    fn handle_challenge_contract_publication_response(
        response: &HostResponse,
        submitted_handle: &Rc<RefCell<Option<SubmittedGameContractPublication>>>,
        local_player_id_handle: &UseStateHandle<Option<[u8; 32]>>,
        lobby_key_handle: &Rc<RefCell<Option<ContractKey>>>,
        api_handle: &Rc<RefCell<Option<WebApi>>>,
        pending_handle: &UseStateHandle<bool>,
        status_handle: &UseStateHandle<String>,
        error_handle: &UseStateHandle<Option<String>>,
    ) -> bool {
        let Some(submitted) = submitted_handle.borrow().clone() else {
            return false;
        };

        let confirmation =
            match confirm_game_contract_publication(response, &submitted.expected_key) {
                Ok(confirmation) => confirmation,

                Err(error) => {
                    submitted_handle.borrow_mut().take();
                    pending_handle.set(true);
                    status_handle.set(
                        "Challenge plan retained after an unexpected contract response".to_owned(),
                    );
                    error_handle.set(Some(error));
                    return true;
                }
            };

        let Some(confirmed_key) = confirmation else {
            return false;
        };

        /*
         * This exact response has now been consumed. Any failure below retains
         * durable signed evidence but cannot reuse the in-memory response token.
         */
        submitted_handle.borrow_mut().take();

        let prepared = (|| {
            let local_player_id = (**local_player_id_handle).ok_or_else(|| {
                "Local identity is unavailable during game-contract confirmation.".to_owned()
            })?;

            let mut stored =
                load_outbound_challenge_publication(&local_player_id)?.ok_or_else(|| {
                    "Exact game-contract confirmation has no durable challenge record.".to_owned()
                })?;

            if stored.signed_offer.body.proposal.game_id != submitted.game_id {
                return Err(
                    "Confirmed game contract does not match the stored signed game ID.".to_owned(),
                );
            }

            let confirmed_contract_id = confirmed_key.id().encode();

            if confirmed_contract_id != submitted.contract_id {
                return Err(
                    "Confirmed game contract ID differs from the armed publication ID.".to_owned(),
                );
            }

            let now_unix_seconds = local_observation_unix_seconds()?;

            verify_challenge_offer_at(
                &stored.signed_offer,
                now_unix_seconds,
            )
            .map_err(|error| {
                format!(
                    "Stored challenge expired or failed verification before lobby publication: {error}"
                )
            })?;

            /*
             * Rebuild before changing durable stage so malformed or inconsistent
             * stored evidence can never become publication-authorized.
             */
            let plan = stored.rebuild_plan()?;

            stored.mark_contract_confirmed()?;
            update_outbound_challenge_publication(&stored)?;

            let lobby_key = lobby_key_handle.borrow().clone().ok_or_else(|| {
                "Game contract is confirmed, but the verified lobby key is unavailable.".to_owned()
            })?;

            Ok((
                lobby_key,
                plan.encoded_lobby_state_update,
                stored.challenge_id(),
            ))
        })();

        let (lobby_key, encoded_lobby_state_update, challenge_id) = match prepared {
            Ok(prepared) => prepared,

            Err(error) => {
                pending_handle.set(true);
                status_handle.set(
                    "Challenge plan retained; lobby advertisement requires recovery".to_owned(),
                );
                error_handle.set(Some(error));
                return true;
            }
        };

        pending_handle.set(true);
        error_handle.set(None);
        status_handle.set("Game contract confirmed; advertising the signed challenge".to_owned());

        let api = api_handle.clone();
        let pending = pending_handle.clone();
        let status = status_handle.clone();
        let error = error_handle.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = {
                let mut api = api.borrow_mut();

                match api.as_mut() {
                    Some(api) => {
                        submit_lobby_state_update(api, lobby_key, encoded_lobby_state_update).await
                    }

                    None => {
                        Err("Freenet connection closed before challenge advertisement.".to_owned())
                    }
                }
            };

            match result {
                Ok(()) => {
                    let short_challenge_id = challenge_id[..5]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();

                    pending.set(true);
                    error.set(None);
                    status.set(format!(
                        "Challenge {short_challenge_id}… submitted; awaiting verified authoritative lobby confirmation",
                    ));
                }

                Err(submission_error) => {
                    /*
                     * The durable stage remains AwaitingLobbyConfirmation.
                     * Recovery must resend the exact rebuilt signed offer.
                     */
                    pending.set(true);
                    status.set(
                        "Game contract confirmed; challenge advertisement requires retry"
                            .to_owned(),
                    );
                    error.set(Some(submission_error));
                }
            }
        });

        true
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
         * Lobby profile state tracks this browser's latest successfully
         * submitted presence intent. Authoritative opponent discovery remains
         * derived exclusively from independently verified lobby state.
         */
        let lobby_display_name = use_state(String::new);
        let lobby_available = use_state(|| false);
        let lobby_presence_submission_pending = use_state(|| false);
        let lobby_profile_status = use_state(|| "Waiting for local identity".to_owned());
        let lobby_profile_error = use_state(|| None::<String>);

        /*
         * Verified authoritative lobby state is separate from local profile
         * intent. It remains absent until the published lobby contract returns
         * a complete state that passes the hostile-input codec.
         */
        let lobby_contract_status = use_state(|| LobbyContractStatus::WaitingForConnection);
        let lobby_subscription_status = use_state(|| SubscriptionStatus::Pending);
        let authoritative_lobby_state = use_state(|| None::<LobbyContractState>);

        /*
         * Outbound challenge publication remains distinct from authoritative
         * lobby state. A durable plan is created before contract publication,
         * and the offer is not advertised until an exact PutResponse is
         * confirmed by the next workflow stage.
         */
        let challenge_publication_pending = use_state(|| false);
        let challenge_publication_status = use_state(|| "No outbound challenge pending".to_owned());
        let challenge_publication_error = use_state(|| None::<String>);
        let submitted_game_contract = use_mut_ref(|| None::<SubmittedGameContractPublication>);

        /*
         * Incoming acceptance begins with volatile unsigned evidence only.
         * The direct read and its originating probe remain inseparable.
         */
        let incoming_acceptance_pending = use_state(|| false);
        let incoming_acceptance_status = use_state(|| {
            "No incoming acceptance pending; contract proof is required before signing".to_owned()
        });
        let incoming_acceptance_error = use_state(|| None::<String>);
        let pending_incoming_acceptance_probe =
            use_mut_ref(|| None::<IncomingChallengeContractProbe>);
        let retrieved_incoming_acceptance_read =
            use_mut_ref(|| None::<(IncomingChallengeContractProbe, ContractKey, Vec<u8>)>);

        /*
         * Outbound recovery is attempted at most once per live connection.
         * A reconnect resets this guard and reuses exact durable evidence.
         */
        let challenge_recovery_attempted = use_mut_ref(|| false);

        /*
         * Incoming acceptance recovery has an independent per-connection
         * guard. It reuses the stored signature and never signs again.
         */
        let incoming_acceptance_recovery_attempted = use_mut_ref(|| false);

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
        let latest_lobby_contract_key = use_mut_ref(|| None::<ContractKey>);

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
            let lobby_display_name = lobby_display_name.clone();
            let lobby_available = lobby_available.clone();
            let lobby_profile_status = lobby_profile_status.clone();
            let lobby_profile_error = lobby_profile_error.clone();

            use_effect_with(*local_player_id, move |player_id| {
                lobby_available.set(false);
                lobby_profile_error.set(None);

                match *player_id {
                    None => {
                        lobby_display_name.set(String::new());
                        lobby_profile_status.set("Waiting for local identity".to_owned());
                    }

                    Some(player_id) => match load_lobby_display_name(&player_id) {
                        Ok(Some(display_name)) => {
                            lobby_display_name.set(display_name);
                            lobby_profile_status.set("Saved display name loaded".to_owned());
                        }

                        Ok(None) => {
                            lobby_display_name.set(String::new());
                            lobby_profile_status.set("Choose a public display name".to_owned());
                        }

                        Err(error) => {
                            lobby_display_name.set(String::new());
                            lobby_profile_error
                                .set(Some(format!("Lobby profile storage error: {error}")));
                            lobby_profile_status.set("Lobby profile unavailable".to_owned());
                        }
                    },
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

        let incoming_acceptance_recovery_ready = (
            *local_player_id,
            matches!(
                &*lobby_contract_status,
                LobbyContractStatus::Retrieved { .. }
            ),
        );

        {
            let freenet_api = freenet_api.clone();
            let latest_lobby_contract_key = latest_lobby_contract_key.clone();
            let incoming_acceptance_recovery_attempted =
                incoming_acceptance_recovery_attempted.clone();
            let incoming_acceptance_pending = incoming_acceptance_pending.clone();
            let incoming_acceptance_status = incoming_acceptance_status.clone();
            let incoming_acceptance_error = incoming_acceptance_error.clone();

            use_effect_with(
                incoming_acceptance_recovery_ready,
                move |(player_id, lobby_ready)| {
                    if let Some(player_id) = *player_id {
                        match load_incoming_challenge_acceptance(&player_id) {
                            Ok(None) => {}

                            Err(error) => {
                                *incoming_acceptance_recovery_attempted.borrow_mut() = true;

                                incoming_acceptance_pending.set(true);
                                incoming_acceptance_status.set(
                                    "Stored incoming acceptance recovery is \
                                     blocked"
                                        .to_owned(),
                                );
                                incoming_acceptance_error.set(Some(error));
                            }

                            Ok(Some(stored)) => {
                                incoming_acceptance_pending.set(true);

                                if !*lobby_ready {
                                    incoming_acceptance_status.set(
                                        "Stored acceptance loaded; waiting for \
                                         verified lobby recovery"
                                            .to_owned(),
                                    );
                                } else if !*incoming_acceptance_recovery_attempted.borrow() {
                                    /*
                                     * Arm before validation or asynchronous
                                     * submission. Any failure retains the
                                     * exact durable record until reconnect.
                                     */
                                    *incoming_acceptance_recovery_attempted.borrow_mut() = true;

                                    let prepared = (|| {
                                        if load_outbound_challenge_publication(&player_id)?
                                            .is_some()
                                        {
                                            return Err("Incoming acceptance recovery \
                                                 is blocked by a durable \
                                                 outbound challenge."
                                                .to_owned());
                                        }

                                        /*
                                         * Rebuilding regenerates the original
                                         * acceptance, proposal, full contract
                                         * key, contract ID, and encoded lobby
                                         * update. It performs no signing.
                                         */
                                        let plan = stored.rebuild_plan()?;

                                        let lobby_key = latest_lobby_contract_key
                                            .borrow()
                                            .clone()
                                            .ok_or_else(|| {
                                                "Verified lobby retrieval \
                                                     did not retain its full \
                                                     contract key."
                                                    .to_owned()
                                            })?;

                                        Ok((
                                            lobby_key,
                                            plan.encoded_lobby_state_update,
                                            stored.challenge_id(),
                                        ))
                                    })();

                                    match prepared {
                                        Err(error) => {
                                            incoming_acceptance_status.set(
                                                "Stored acceptance failed \
                                                 exact recovery validation"
                                                    .to_owned(),
                                            );
                                            incoming_acceptance_error.set(Some(error));
                                        }

                                        Ok((
                                            lobby_key,
                                            encoded_lobby_state_update,
                                            challenge_id,
                                        )) => {
                                            let short_challenge_id = challenge_id[..5]
                                                .iter()
                                                .map(|byte| format!("{byte:02x}"))
                                                .collect::<String>();

                                            incoming_acceptance_status.set(format!(
                                                "Recovering acceptance for \
                                                     challenge \
                                                     {short_challenge_id}…",
                                            ));
                                            incoming_acceptance_error.set(None);

                                            let api = freenet_api.clone();
                                            let pending = incoming_acceptance_pending.clone();
                                            let status = incoming_acceptance_status.clone();
                                            let error = incoming_acceptance_error.clone();

                                            wasm_bindgen_futures::spawn_local(async move {
                                                let result = {
                                                    let mut api = api.borrow_mut();

                                                    match api.as_mut() {
                                                        Some(api) => {
                                                            submit_lobby_state_update(
                                                                api,
                                                                lobby_key,
                                                                encoded_lobby_state_update,
                                                            )
                                                            .await
                                                        }

                                                        None => Err("Freenet \
                                                                 connection \
                                                                 closed before \
                                                                 incoming \
                                                                 acceptance \
                                                                 recovery."
                                                            .to_owned()),
                                                    }
                                                };

                                                match result {
                                                    Ok(()) => {
                                                        pending.set(true);
                                                        error.set(None);
                                                        status.set(format!(
                                                            "Recovered \
                                                                 acceptance for \
                                                                 challenge \
                                                                 {short_challenge_id}… \
                                                                 submitted; \
                                                                 awaiting verified \
                                                                 authoritative \
                                                                 confirmation",
                                                        ));
                                                    }

                                                    Err(submit_error) => {
                                                        pending.set(true);
                                                        status.set(
                                                            "Stored \
                                                                 acceptance \
                                                                 retained; \
                                                                 recovery \
                                                                 submission \
                                                                 failed"
                                                                .to_owned(),
                                                        );
                                                        error.set(Some(submit_error));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    || {}
                },
            );
        }

        let challenge_recovery_ready = (
            *local_player_id,
            matches!(
                &*lobby_contract_status,
                LobbyContractStatus::Retrieved { .. }
            ),
        );

        {
            let freenet_api = freenet_api.clone();
            let submitted_game_contract = submitted_game_contract.clone();
            let challenge_recovery_attempted = challenge_recovery_attempted.clone();
            let challenge_publication_pending = challenge_publication_pending.clone();
            let challenge_publication_status = challenge_publication_status.clone();
            let challenge_publication_error = challenge_publication_error.clone();

            use_effect_with(challenge_recovery_ready, move |(player_id, lobby_ready)| {
                if let Some(player_id) = *player_id {
                    match load_outbound_challenge_publication(&player_id) {
                        Ok(None) => {}

                        Err(error) => {
                            *challenge_recovery_attempted.borrow_mut() = true;

                            challenge_publication_pending.set(true);
                            challenge_publication_status
                                .set("Stored challenge recovery is blocked".to_owned());
                            challenge_publication_error.set(Some(error));
                        }

                        Ok(Some(stored)) => {
                            challenge_publication_pending.set(true);

                            if !*lobby_ready {
                                challenge_publication_status.set(
                                    "Stored challenge loaded; waiting for verified lobby recovery"
                                        .to_owned(),
                                );
                            } else {
                                let already_attempted = *challenge_recovery_attempted.borrow()
                                    || submitted_game_contract.borrow().is_some();

                                if !already_attempted {
                                    *challenge_recovery_attempted.borrow_mut() = true;

                                    match local_observation_unix_seconds() {
                                        Err(error) => {
                                            challenge_publication_status
                                                    .set(
                                                        "Stored challenge recovery requires a valid browser clock"
                                                            .to_owned(),
                                                    );
                                            challenge_publication_error.set(Some(error));
                                        }

                                        Ok(now_unix_seconds)
                                            if now_unix_seconds
                                                >= stored
                                                    .signed_offer
                                                    .body
                                                    .expires_at_unix_seconds =>
                                        {
                                            match remove_outbound_challenge_publication(
                                                &player_id,
                                                &stored.challenge_id(),
                                            ) {
                                                Ok(()) => {
                                                    challenge_publication_pending.set(false);
                                                    challenge_publication_status
                                                            .set(
                                                                "Expired stored challenge removed; ready for a new challenge"
                                                                    .to_owned(),
                                                            );
                                                    challenge_publication_error.set(None);
                                                }

                                                Err(error) => {
                                                    challenge_publication_status
                                                            .set(
                                                                "Expired challenge cleanup requires recovery"
                                                                    .to_owned(),
                                                            );
                                                    challenge_publication_error.set(Some(error));
                                                }
                                            }
                                        }

                                        Ok(now_unix_seconds) => {
                                            let prepared = (|| {
                                                verify_challenge_offer_at(
                                                        &stored.signed_offer,
                                                        now_unix_seconds,
                                                    )
                                                    .map_err(|error| {
                                                        format!(
                                                            "Stored challenge failed live recovery verification: {error}"
                                                        )
                                                    })?;

                                                let plan = stored.rebuild_plan()?;

                                                if plan.contract_publication.game_id
                                                    != stored.signed_offer.body.proposal.game_id
                                                {
                                                    return Err(
                                                            "Recovered challenge game ID does not match its contract publication."
                                                                .to_owned(),
                                                        );
                                                }

                                                Ok((
                                                    plan.contract_publication.game_id,
                                                    stored.challenge_id(),
                                                ))
                                            })(
                                            );

                                            match prepared {
                                                Err(error) => {
                                                    challenge_publication_status
                                                            .set(
                                                                "Stored challenge failed exact recovery validation"
                                                                    .to_owned(),
                                                            );
                                                    challenge_publication_error.set(Some(error));
                                                }

                                                Ok((game_id, challenge_id)) => {
                                                    let short_challenge_id = challenge_id[..5]
                                                        .iter()
                                                        .map(|byte| format!("{byte:02x}"))
                                                        .collect::<String>();

                                                    challenge_publication_status
                                                            .set(format!(
                                                                "Recovering challenge {short_challenge_id}… by re-confirming its exact game contract",
                                                            ));
                                                    challenge_publication_error.set(None);

                                                    let api = freenet_api.clone();
                                                    let submitted = submitted_game_contract.clone();
                                                    let pending =
                                                        challenge_publication_pending.clone();
                                                    let status =
                                                        challenge_publication_status.clone();
                                                    let error = challenge_publication_error.clone();

                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        let result = {
                                                            let mut api = api.borrow_mut();

                                                            match api
                                                                        .as_mut()
                                                                    {
                                                                        Some(api) => {
                                                                            let submitted_before_send =
                                                                                submitted.clone();

                                                                            submit_game_contract_publication(
                                                                                api,
                                                                                game_id,
                                                                                move |prepared| {
                                                                                    *submitted_before_send
                                                                                        .borrow_mut() =
                                                                                        prepared.cloned();
                                                                                },
                                                                            )
                                                                            .await
                                                                        }

                                                                        None => Err(
                                                                            "Freenet connection closed before stored challenge recovery."
                                                                                .to_owned(),
                                                                        ),
                                                                    }
                                                        };

                                                        match result {
                                                            Ok(publication) => {
                                                                let short_contract_id = publication
                                                                    .contract_id
                                                                    .chars()
                                                                    .take(10)
                                                                    .collect::<String>();

                                                                pending.set(true);
                                                                error.set(None);
                                                                status
                                                                            .set(format!(
                                                                                "Recovered game contract {short_contract_id}… submitted; awaiting exact confirmation",
                                                                            ));
                                                            }

                                                            Err(recovery_error) => {
                                                                pending.set(true);
                                                                status
                                                                            .set(
                                                                                "Stored challenge retained; contract recovery requires reconnect"
                                                                                    .to_owned(),
                                                                            );
                                                                error.set(Some(recovery_error));
                                                            }
                                                        }
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

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
            let latest_lobby_contract_key = latest_lobby_contract_key.clone();
            let lobby_contract_status = lobby_contract_status.clone();
            let lobby_subscription_status = lobby_subscription_status.clone();
            let authoritative_lobby_state = authoritative_lobby_state.clone();
            let challenge_publication_pending = challenge_publication_pending.clone();
            let challenge_publication_status = challenge_publication_status.clone();
            let challenge_publication_error = challenge_publication_error.clone();
            let submitted_game_contract = submitted_game_contract.clone();
            let incoming_acceptance_pending = incoming_acceptance_pending.clone();
            let incoming_acceptance_status = incoming_acceptance_status.clone();
            let incoming_acceptance_error = incoming_acceptance_error.clone();
            let pending_incoming_acceptance_probe = pending_incoming_acceptance_probe.clone();
            let retrieved_incoming_acceptance_read = retrieved_incoming_acceptance_read.clone();
            let challenge_recovery_attempted = challenge_recovery_attempted.clone();
            let incoming_acceptance_recovery_attempted =
                incoming_acceptance_recovery_attempted.clone();
            let local_player_id_for_effect = local_player_id.clone();
            let authoritative_local_role_for_effect = authoritative_local_role.clone();
            let controller_for_effect = controller.clone();

            use_effect_with((), move |_| {
                /*
                 * Replacing the role dependency replaces the active transport
                 * closure. Any durable pending action remains in storage.
                 */
                freenet_api.borrow_mut().take();
                submitted_game_contract.borrow_mut().take();
                pending_incoming_acceptance_probe.borrow_mut().take();
                retrieved_incoming_acceptance_read.borrow_mut().take();
                incoming_acceptance_pending.set(false);
                incoming_acceptance_status.set(
                    "No incoming acceptance pending; contract proof is required before signing"
                        .to_owned(),
                );
                incoming_acceptance_error.set(None);
                *challenge_recovery_attempted.borrow_mut() = false;
                *incoming_acceptance_recovery_attempted.borrow_mut() = false;
                *local_network_action_submitted.borrow_mut() = None;
                *local_dice_secret.borrow_mut() = None;
                latest_contract_key.borrow_mut().take();
                latest_authoritative_state.borrow_mut().take();
                latest_lobby_contract_key.borrow_mut().take();
                authoritative_local_role_for_effect.set(None);

                let status_for_callback = connection_status.clone();
                let contract_for_response = contract_status.clone();
                let contract_for_host_error = contract_status.clone();
                let lobby_contract_for_response = lobby_contract_status.clone();
                let lobby_contract_for_host_error = lobby_contract_status.clone();
                let subscription_for_response = subscription_status.clone();
                let subscription_for_status = subscription_status.clone();
                let lobby_subscription_for_response = lobby_subscription_status.clone();
                let lobby_subscription_for_status = lobby_subscription_status.clone();
                let api_for_response = freenet_api.clone();
                let network_action_for_response = local_network_action_submitted.clone();
                let secret_for_response = local_dice_secret.clone();
                let secret_status_for_response = dice_secret_status.clone();
                let key_for_response = latest_contract_key.clone();
                let state_for_response = latest_authoritative_state.clone();
                let lobby_state_for_response = authoritative_lobby_state.clone();
                let lobby_key_for_response = latest_lobby_contract_key.clone();
                let challenge_pending_for_response = challenge_publication_pending.clone();
                let challenge_status_for_response = challenge_publication_status.clone();
                let challenge_error_for_response = challenge_publication_error.clone();
                let submitted_game_for_response = submitted_game_contract.clone();
                let incoming_probe_for_response = pending_incoming_acceptance_probe.clone();
                let incoming_read_for_response = retrieved_incoming_acceptance_read.clone();
                let incoming_pending_for_response = incoming_acceptance_pending.clone();
                let incoming_status_for_response = incoming_acceptance_status.clone();
                let incoming_error_for_response = incoming_acceptance_error.clone();
                let player_id_for_response = local_player_id_for_effect.clone();
                let authoritative_role_for_response = authoritative_local_role_for_effect.clone();
                let controller_for_response = controller_for_effect.clone();

                let incoming_probe_for_status = pending_incoming_acceptance_probe.clone();
                let incoming_read_for_status = retrieved_incoming_acceptance_read.clone();
                let incoming_pending_for_status = incoming_acceptance_pending.clone();
                let incoming_status_for_status = incoming_acceptance_status.clone();
                let incoming_error_for_status = incoming_acceptance_error.clone();

                let api_for_open = freenet_api.clone();
                let connection_for_open = connection_status.clone();
                let contract_for_open = contract_status.clone();
                let subscription_for_open = subscription_status.clone();
                let lobby_contract_for_open = lobby_contract_status.clone();
                let lobby_subscription_for_open = lobby_subscription_status.clone();

                match connect(
                    move |status| {
                        match &status {
                            ConnectionStatus::Connecting => {
                                subscription_for_status.set(SubscriptionStatus::Pending);
                                lobby_subscription_for_status.set(SubscriptionStatus::Pending);
                            }
                            ConnectionStatus::Connected => {}
                            ConnectionStatus::Disconnected | ConnectionStatus::Failed(_) => {
                                subscription_for_status.set(SubscriptionStatus::Inactive);
                                lobby_subscription_for_status.set(SubscriptionStatus::Inactive);

                                let had_volatile_incoming_evidence =
                                    incoming_probe_for_status.borrow().is_some()
                                        || incoming_read_for_status.borrow().is_some();

                                incoming_probe_for_status.borrow_mut().take();
                                incoming_read_for_status.borrow_mut().take();

                                if had_volatile_incoming_evidence {
                                    incoming_pending_for_status.set(false);
                                    incoming_status_for_status.set(
                                        "Incoming contract proof was interrupted;                                          retry after reconnecting"
                                            .to_owned(),
                                    );
                                    incoming_error_for_status.set(Some(
                                        "Volatile unsigned acceptance evidence                                          was cleared when the connection closed."
                                            .to_owned(),
                                    ));
                                }
                            }
                        }

                        status_for_callback.set(status);
                    },
                    move |response| {
                        if handle_lobby_response(
                            &response,
                            &lobby_contract_for_response,
                            &lobby_subscription_for_response,
                            &lobby_state_for_response,
                            &lobby_key_for_response,
                            &api_for_response,
                            &player_id_for_response,
                            &challenge_pending_for_response,
                            &challenge_status_for_response,
                            &challenge_error_for_response,
                            &incoming_pending_for_response,
                            &incoming_status_for_response,
                            &incoming_error_for_response,
                        ) {
                            return;
                        }

                        if handle_incoming_challenge_contract_response(
                            &response,
                            &incoming_probe_for_response,
                            &incoming_read_for_response,
                            &lobby_state_for_response,
                            &player_id_for_response,
                            &lobby_key_for_response,
                            &api_for_response,
                            &incoming_pending_for_response,
                            &incoming_status_for_response,
                            &incoming_error_for_response,
                        ) {
                            return;
                        }

                        if handle_challenge_contract_publication_response(
                            &response,
                            &submitted_game_for_response,
                            &player_id_for_response,
                            &lobby_key_for_response,
                            &api_for_response,
                            &challenge_pending_for_response,
                            &challenge_status_for_response,
                            &challenge_error_for_response,
                        ) {
                            return;
                        }

                        if let Some(classified) = classify_response(response, TEST_CONTRACT_ID) {
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
                                                                TEST_CONTRACT_ID,
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
                                                                    request_contract(api, TEST_CONTRACT_ID)
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
                                                        submit_action_delta(api, TEST_CONTRACT_ID, key, delta).await
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
                                                                request_contract(api, TEST_CONTRACT_ID).await
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

                        lobby_contract_for_host_error
                            .set(crate::lobby_transport::host_result_error_status(&error));
                    },
                    move || {
                        let api_for_request = api_for_open.clone();
                        let connection_for_request = connection_for_open.clone();
                        let contract_for_request = contract_for_open.clone();
                        let subscription_for_request = subscription_for_open.clone();
                        let lobby_contract_for_request = lobby_contract_for_open.clone();
                        let lobby_subscription_for_request = lobby_subscription_for_open.clone();

                        wasm_bindgen_futures::spawn_local(async move {
                            contract_for_request.set(ContractProbeStatus::Requesting);
                            subscription_for_request.set(SubscriptionStatus::Pending);

                            lobby_contract_for_request.set(LobbyContractStatus::Requesting);
                            lobby_subscription_for_request.set(SubscriptionStatus::Pending);

                            let (game_result, lobby_result) = {
                                let mut api = api_for_request.borrow_mut();

                                match api.as_mut() {
                                    Some(api) => {
                                        let game_result =
                                            request_contract(api, TEST_CONTRACT_ID).await;
                                        let lobby_result =
                                            request_lobby_contract(api).await;

                                        (game_result, lobby_result)
                                    }

                                    None => (
                                        Err(
                                            "Freenet WebSocket opened without an active API handle."
                                                .to_owned(),
                                        ),
                                        Err(
                                            "Freenet WebSocket opened without an active API handle."
                                                .to_owned(),
                                        ),
                                    ),
                                }
                            };

                            if let Err(error) = game_result {
                                connection_for_request.set(ConnectionStatus::Failed(error.clone()));
                                contract_for_request.set(ContractProbeStatus::Failed(error));
                                subscription_for_request.set(SubscriptionStatus::Inactive);
                            }

                            if let Err(error) = lobby_result {
                                lobby_contract_for_request.set(LobbyContractStatus::Failed(error));
                                lobby_subscription_for_request.set(SubscriptionStatus::Inactive);
                            }
                        });
                    },
                ) {
                    Ok(api) => {
                        *freenet_api.borrow_mut() = Some(api);
                    }
                    Err(error) => {
                        connection_status.set(ConnectionStatus::Failed(error.clone()));
                        lobby_contract_status.set(LobbyContractStatus::Failed(error));
                        lobby_subscription_status.set(SubscriptionStatus::Inactive);
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
                                Some(api) => {
                                    submit_action_delta(api, TEST_CONTRACT_ID, key, delta).await
                                }

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
                                        Some(api) => request_contract(api, TEST_CONTRACT_ID).await,

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
            let latest_lobby_contract_key = latest_lobby_contract_key.clone();
            let lobby_contract_status = lobby_contract_status.clone();
            let lobby_subscription_status = lobby_subscription_status.clone();
            let authoritative_lobby_state = authoritative_lobby_state.clone();
            let challenge_publication_pending = challenge_publication_pending.clone();
            let challenge_publication_status = challenge_publication_status.clone();
            let challenge_publication_error = challenge_publication_error.clone();
            let submitted_game_contract = submitted_game_contract.clone();
            let incoming_acceptance_pending = incoming_acceptance_pending.clone();
            let incoming_acceptance_status = incoming_acceptance_status.clone();
            let incoming_acceptance_error = incoming_acceptance_error.clone();
            let pending_incoming_acceptance_probe = pending_incoming_acceptance_probe.clone();
            let retrieved_incoming_acceptance_read = retrieved_incoming_acceptance_read.clone();
            let challenge_recovery_attempted = challenge_recovery_attempted.clone();
            let incoming_acceptance_recovery_attempted =
                incoming_acceptance_recovery_attempted.clone();
            let local_player_id_for_reconnect = local_player_id.clone();
            let authoritative_local_role_for_reconnect = authoritative_local_role.clone();
            let controller_for_reconnect = controller.clone();

            Callback::from(move |_| {
                freenet_api.borrow_mut().take();
                submitted_game_contract.borrow_mut().take();
                pending_incoming_acceptance_probe.borrow_mut().take();
                retrieved_incoming_acceptance_read.borrow_mut().take();
                incoming_acceptance_pending.set(false);
                incoming_acceptance_status.set(
                    "No incoming acceptance pending; contract proof is required before signing"
                        .to_owned(),
                );
                incoming_acceptance_error.set(None);
                *challenge_recovery_attempted.borrow_mut() = false;
                *incoming_acceptance_recovery_attempted.borrow_mut() = false;

                /*
                 * Permit one submission attempt on the new connection.
                 * The durable pending action itself remains unchanged.
                 */
                *local_network_action_submitted.borrow_mut() = None;
                latest_contract_key.borrow_mut().take();
                latest_authoritative_state.borrow_mut().take();
                latest_lobby_contract_key.borrow_mut().take();
                authoritative_local_role_for_reconnect.set(None);

                contract_status.set(ContractProbeStatus::WaitingForConnection);
                subscription_status.set(SubscriptionStatus::Pending);
                lobby_contract_status.set(LobbyContractStatus::WaitingForConnection);
                lobby_subscription_status.set(SubscriptionStatus::Pending);
                dice_secret_status.set("Checking browser storage".to_owned());

                let status_for_callback = connection_status.clone();
                let contract_for_response = contract_status.clone();
                let contract_for_host_error = contract_status.clone();
                let lobby_contract_for_response = lobby_contract_status.clone();
                let lobby_contract_for_host_error = lobby_contract_status.clone();
                let subscription_for_response = subscription_status.clone();
                let subscription_for_status = subscription_status.clone();
                let lobby_subscription_for_response = lobby_subscription_status.clone();
                let lobby_subscription_for_status = lobby_subscription_status.clone();
                let api_for_response = freenet_api.clone();
                let network_action_for_response = local_network_action_submitted.clone();
                let secret_for_response = local_dice_secret.clone();
                let secret_status_for_response = dice_secret_status.clone();
                let key_for_response = latest_contract_key.clone();
                let state_for_response = latest_authoritative_state.clone();
                let lobby_state_for_response = authoritative_lobby_state.clone();
                let lobby_key_for_response = latest_lobby_contract_key.clone();
                let challenge_pending_for_response = challenge_publication_pending.clone();
                let challenge_status_for_response = challenge_publication_status.clone();
                let challenge_error_for_response = challenge_publication_error.clone();
                let submitted_game_for_response = submitted_game_contract.clone();
                let incoming_probe_for_response = pending_incoming_acceptance_probe.clone();
                let incoming_read_for_response = retrieved_incoming_acceptance_read.clone();
                let incoming_pending_for_response = incoming_acceptance_pending.clone();
                let incoming_status_for_response = incoming_acceptance_status.clone();
                let incoming_error_for_response = incoming_acceptance_error.clone();
                let player_id_for_response = local_player_id_for_reconnect.clone();
                let authoritative_role_for_response =
                    authoritative_local_role_for_reconnect.clone();
                let controller_for_response = controller_for_reconnect.clone();

                let incoming_probe_for_status = pending_incoming_acceptance_probe.clone();
                let incoming_read_for_status = retrieved_incoming_acceptance_read.clone();
                let incoming_pending_for_status = incoming_acceptance_pending.clone();
                let incoming_status_for_status = incoming_acceptance_status.clone();
                let incoming_error_for_status = incoming_acceptance_error.clone();

                let api_for_open = freenet_api.clone();
                let connection_for_open = connection_status.clone();
                let contract_for_open = contract_status.clone();
                let subscription_for_open = subscription_status.clone();
                let lobby_contract_for_open = lobby_contract_status.clone();
                let lobby_subscription_for_open = lobby_subscription_status.clone();

                match connect(
                    move |status| {
                        match &status {
                            ConnectionStatus::Connecting => {
                                subscription_for_status.set(SubscriptionStatus::Pending);
                                lobby_subscription_for_status.set(SubscriptionStatus::Pending);
                            }
                            ConnectionStatus::Connected => {}
                            ConnectionStatus::Disconnected | ConnectionStatus::Failed(_) => {
                                subscription_for_status.set(SubscriptionStatus::Inactive);
                                lobby_subscription_for_status.set(SubscriptionStatus::Inactive);

                                let had_volatile_incoming_evidence =
                                    incoming_probe_for_status.borrow().is_some()
                                        || incoming_read_for_status.borrow().is_some();

                                incoming_probe_for_status.borrow_mut().take();
                                incoming_read_for_status.borrow_mut().take();

                                if had_volatile_incoming_evidence {
                                    incoming_pending_for_status.set(false);
                                    incoming_status_for_status.set(
                                        "Incoming contract proof was interrupted;                                          retry after reconnecting"
                                            .to_owned(),
                                    );
                                    incoming_error_for_status.set(Some(
                                        "Volatile unsigned acceptance evidence                                          was cleared when the connection closed."
                                            .to_owned(),
                                    ));
                                }
                            }
                        }

                        status_for_callback.set(status);
                    },
                    move |response| {
                        if handle_lobby_response(
                            &response,
                            &lobby_contract_for_response,
                            &lobby_subscription_for_response,
                            &lobby_state_for_response,
                            &lobby_key_for_response,
                            &api_for_response,
                            &player_id_for_response,
                            &challenge_pending_for_response,
                            &challenge_status_for_response,
                            &challenge_error_for_response,
                            &incoming_pending_for_response,
                            &incoming_status_for_response,
                            &incoming_error_for_response,
                        ) {
                            return;
                        }

                        if handle_incoming_challenge_contract_response(
                            &response,
                            &incoming_probe_for_response,
                            &incoming_read_for_response,
                            &lobby_state_for_response,
                            &player_id_for_response,
                            &lobby_key_for_response,
                            &api_for_response,
                            &incoming_pending_for_response,
                            &incoming_status_for_response,
                            &incoming_error_for_response,
                        ) {
                            return;
                        }

                        if handle_challenge_contract_publication_response(
                            &response,
                            &submitted_game_for_response,
                            &player_id_for_response,
                            &lobby_key_for_response,
                            &api_for_response,
                            &challenge_pending_for_response,
                            &challenge_status_for_response,
                            &challenge_error_for_response,
                        ) {
                            return;
                        }

                        if let Some(classified) = classify_response(response, TEST_CONTRACT_ID) {
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
                                                                TEST_CONTRACT_ID,
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
                                                                    request_contract(api, TEST_CONTRACT_ID)
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
                                                        submit_action_delta(api, TEST_CONTRACT_ID, key, delta).await
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
                                                                request_contract(api, TEST_CONTRACT_ID).await
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

                        lobby_contract_for_host_error
                            .set(crate::lobby_transport::host_result_error_status(&error));
                    },
                    move || {
                        let api_for_request = api_for_open.clone();
                        let connection_for_request = connection_for_open.clone();
                        let contract_for_request = contract_for_open.clone();
                        let subscription_for_request = subscription_for_open.clone();
                        let lobby_contract_for_request = lobby_contract_for_open.clone();
                        let lobby_subscription_for_request = lobby_subscription_for_open.clone();

                        wasm_bindgen_futures::spawn_local(async move {
                            contract_for_request.set(ContractProbeStatus::Requesting);
                            subscription_for_request.set(SubscriptionStatus::Pending);

                            lobby_contract_for_request.set(LobbyContractStatus::Requesting);
                            lobby_subscription_for_request.set(SubscriptionStatus::Pending);

                            let (game_result, lobby_result) = {
                                let mut api = api_for_request.borrow_mut();

                                match api.as_mut() {
                                    Some(api) => {
                                        let game_result =
                                            request_contract(api, TEST_CONTRACT_ID).await;
                                        let lobby_result =
                                            request_lobby_contract(api).await;

                                        (game_result, lobby_result)
                                    }

                                    None => (
                                        Err(
                                            "Freenet WebSocket opened without an active API handle."
                                                .to_owned(),
                                        ),
                                        Err(
                                            "Freenet WebSocket opened without an active API handle."
                                                .to_owned(),
                                        ),
                                    ),
                                }
                            };

                            if let Err(error) = game_result {
                                connection_for_request.set(ConnectionStatus::Failed(error.clone()));
                                contract_for_request.set(ContractProbeStatus::Failed(error));
                                subscription_for_request.set(SubscriptionStatus::Inactive);
                            }

                            if let Err(error) = lobby_result {
                                lobby_contract_for_request.set(LobbyContractStatus::Failed(error));
                                lobby_subscription_for_request.set(SubscriptionStatus::Inactive);
                            }
                        });
                    },
                ) {
                    Ok(api) => {
                        *freenet_api.borrow_mut() = Some(api);
                    }
                    Err(error) => {
                        connection_status.set(ConnectionStatus::Failed(error.clone()));
                        lobby_contract_status.set(LobbyContractStatus::Failed(error));
                        lobby_subscription_status.set(SubscriptionStatus::Inactive);
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

        let on_lobby_name_input = {
            let lobby_display_name = lobby_display_name.clone();
            let lobby_available = lobby_available.clone();
            let lobby_profile_status = lobby_profile_status.clone();
            let lobby_profile_error = lobby_profile_error.clone();

            Callback::from(move |event: yew::events::InputEvent| {
                let input: web_sys::HtmlInputElement = event.target_unchecked_into();

                lobby_display_name.set(input.value());
                lobby_available.set(false);
                lobby_profile_status.set("Unsaved display name; currently unavailable".to_owned());
                lobby_profile_error.set(None);
            })
        };

        let on_save_lobby_name = {
            let local_player_id = local_player_id.clone();
            let lobby_display_name = lobby_display_name.clone();
            let lobby_profile_status = lobby_profile_status.clone();
            let lobby_profile_error = lobby_profile_error.clone();

            Callback::from(move |_| {
                let Some(player_id) = *local_player_id else {
                    lobby_profile_error.set(Some("Local identity is not ready yet.".to_owned()));
                    return;
                };

                match store_lobby_display_name(&player_id, lobby_display_name.as_str()) {
                    Ok(()) => {
                        lobby_profile_error.set(None);
                        lobby_profile_status.set("Display name saved locally".to_owned());
                    }

                    Err(error) => {
                        lobby_profile_error.set(Some(error));
                    }
                }
            })
        };

        let on_toggle_lobby_availability = {
            let local_player_id = local_player_id.clone();
            let lobby_display_name = lobby_display_name.clone();
            let lobby_available = lobby_available.clone();
            let lobby_presence_submission_pending = lobby_presence_submission_pending.clone();
            let lobby_profile_status = lobby_profile_status.clone();
            let lobby_profile_error = lobby_profile_error.clone();
            let latest_lobby_contract_key = latest_lobby_contract_key.clone();
            let freenet_api = freenet_api.clone();

            Callback::from(move |_| {
                if *lobby_presence_submission_pending {
                    return;
                }

                let target_available = !*lobby_available;

                let prepared = (|| {
                    let player_id = (*local_player_id)
                        .ok_or_else(|| "Local identity is not ready yet.".to_owned())?;

                    store_lobby_display_name(&player_id, lobby_display_name.as_str())?;

                    let signing_key = load_local_identity()?
                        .ok_or_else(|| "Stored local identity is unavailable.".to_owned())?;

                    if player_id_for_signing_key(&signing_key) != player_id {
                        return Err(
                            "Stored signing identity does not match the active PlayerId."
                                .to_owned(),
                        );
                    }

                    let issued_at_unix_seconds = local_observation_unix_seconds()?;

                    let key = latest_lobby_contract_key
                        .borrow()
                        .as_ref()
                        .cloned()
                        .ok_or_else(|| {
                            "The verified lobby contract key is not available yet.".to_owned()
                        })?;

                    if freenet_api.borrow().is_none() {
                        return Err("The Freenet connection is not available.".to_owned());
                    }

                    /*
                     * Reserve before constructing or signing. A failure after
                     * this point may skip a revision, which is safe; reusing one
                     * would not be.
                     */
                    let revision = reserve_next_presence_revision(&player_id)?;

                    let plan = plan_lobby_presence(LobbyPresencePlannerInput {
                        signing_key: &signing_key,
                        display_name: lobby_display_name.as_str(),
                        available: target_available,
                        revision,
                        issued_at_unix_seconds,
                    })?;

                    Ok((key, plan.encoded_state_update, revision))
                })();

                let (key, state_update, revision) = match prepared {
                    Ok(prepared) => prepared,

                    Err(error) => {
                        lobby_profile_error.set(Some(error));
                        return;
                    }
                };

                lobby_presence_submission_pending.set(true);
                lobby_profile_error.set(None);
                lobby_profile_status.set(format!(
                    "Publishing {} presence revision {revision}",
                    if target_available {
                        "available"
                    } else {
                        "unavailable"
                    },
                ));

                let api_for_update = freenet_api.clone();
                let available_for_update = lobby_available.clone();
                let pending_for_update = lobby_presence_submission_pending.clone();
                let status_for_update = lobby_profile_status.clone();
                let error_for_update = lobby_profile_error.clone();

                wasm_bindgen_futures::spawn_local(async move {
                    let submit_result = {
                        let mut api = api_for_update.borrow_mut();

                        match api.as_mut() {
                            Some(api) => submit_lobby_state_update(api, key, state_update).await,

                            None => {
                                Err("Freenet connection closed before presence publication."
                                    .to_owned())
                            }
                        }
                    };

                    match submit_result {
                        Ok(()) => {
                            available_for_update.set(target_available);
                            status_for_update.set(format!(
                                "{} presence revision {revision} submitted; awaiting verified lobby refresh",
                                if target_available {
                                    "Available"
                                } else {
                                    "Unavailable"
                                },
                            ));

                            gloo_timers::future::TimeoutFuture::new(750).await;

                            let refresh_result = {
                                let mut api = api_for_update.borrow_mut();

                                match api.as_mut() {
                                    Some(api) => request_lobby_contract(api).await,

                                    None => {
                                        Err("Freenet connection closed before lobby verification."
                                            .to_owned())
                                    }
                                }
                            };

                            if let Err(error) = refresh_result {
                                error_for_update.set(Some(format!(
                                    "Presence revision {revision} was submitted, but lobby refresh failed: {error}"
                                )));
                            }
                        }

                        Err(error) => {
                            error_for_update.set(Some(error));
                            status_for_update.set("Presence publication failed".to_owned());
                        }
                    }

                    pending_for_update.set(false);
                });
            })
        };

        let on_challenge_player: Callback<([u8; 32], String, u64)> = {
            let local_player_id = local_player_id.clone();
            let lobby_display_name = lobby_display_name.clone();
            let latest_lobby_contract_key = latest_lobby_contract_key.clone();
            let freenet_api = freenet_api.clone();
            let challenge_publication_pending = challenge_publication_pending.clone();
            let challenge_publication_status = challenge_publication_status.clone();
            let challenge_publication_error = challenge_publication_error.clone();
            let submitted_game_contract = submitted_game_contract.clone();

            Callback::from(
                move |(recipient_id, recipient_display_name, recipient_presence_expiry): (
                    [u8; 32],
                    String,
                    u64,
                )| {
                    if *challenge_publication_pending {
                        return;
                    }

                    let prepared = (|| {
                        let local_player_id = (*local_player_id)
                            .ok_or_else(|| "Local identity is not ready yet.".to_owned())?;

                        let signing_key = load_local_identity()?
                            .ok_or_else(|| "Stored local identity is unavailable.".to_owned())?;

                        if player_id_for_signing_key(&signing_key) != local_player_id {
                            return Err(
                                "Stored signing identity does not match the active PlayerId."
                                    .to_owned(),
                            );
                        }

                        let now = local_observation_unix_seconds()?;

                        if now >= recipient_presence_expiry {
                            return Err(
                                "The selected opponent's availability has expired; refresh the lobby."
                                    .to_owned(),
                            );
                        }

                        if latest_lobby_contract_key.borrow().is_none() {
                            return Err(
                                "The verified lobby contract key is not available yet.".to_owned()
                            );
                        }

                        if freenet_api.borrow().is_none() {
                            return Err("The Freenet connection is not available.".to_owned());
                        }

                        if load_outbound_challenge_publication(&local_player_id)?.is_some() {
                            return Err(
                                "A durable outbound challenge is already pending for this identity."
                                    .to_owned(),
                            );
                        }

                        let expires_at_unix_seconds = now
                            .checked_add(600)
                            .ok_or_else(|| "Challenge expiration overflowed.".to_owned())?;

                        /*
                         * These are separate calls deliberately. The planner
                         * rejects zero or reused identifiers even if browser
                         * randomness were ever to return an impossible
                         * collision.
                         */
                        let challenge_id = secure_random_32("challenge ID")?;
                        let game_id = secure_random_32("game ID")?;
                        let genesis_action_id = secure_random_32("genesis action ID")?;

                        let plan = plan_outbound_challenge(OutboundChallengePlannerInput {
                            signing_key: &signing_key,
                            challenger_display_name: lobby_display_name.as_str(),
                            recipient_id,
                            recipient_display_name: &recipient_display_name,
                            match_length: 1,
                            challenge_id,
                            game_id,
                            genesis_action_id,
                            created_at_unix_seconds: now,
                            expires_at_unix_seconds,
                        })?;

                        let stored = StoredOutboundChallengePublication::new(&plan)?;

                        /*
                         * This exact read-back-verified write must complete
                         * before the game contract is submitted.
                         */
                        store_new_outbound_challenge_publication(&stored)?;

                        Ok(game_id)
                    })();

                    let game_id = match prepared {
                        Ok(game_id) => game_id,

                        Err(error) => {
                            challenge_publication_error.set(Some(error));
                            return;
                        }
                    };

                    challenge_publication_pending.set(true);
                    challenge_publication_error.set(None);
                    challenge_publication_status.set(format!(
                        "Publishing a single-game contract before advertising the challenge to {recipient_display_name}",
                    ));

                    let api_for_publication = freenet_api.clone();
                    let pending_for_publication = challenge_publication_pending.clone();
                    let status_for_publication = challenge_publication_status.clone();
                    let error_for_publication = challenge_publication_error.clone();
                    let submitted_for_publication = submitted_game_contract.clone();

                    wasm_bindgen_futures::spawn_local(async move {
                        let result = {
                            let mut api = api_for_publication.borrow_mut();

                            match api.as_mut() {
                                Some(api) => {
                                    let submitted_before_send = submitted_for_publication.clone();

                                    submit_game_contract_publication(
                                        api,
                                        game_id,
                                        move |submitted| {
                                            *submitted_before_send.borrow_mut() =
                                                submitted.cloned();
                                        },
                                    )
                                    .await
                                }

                                None => Err(
                                    "Freenet connection closed before game-contract publication."
                                        .to_owned(),
                                ),
                            }
                        };

                        match result {
                            Ok(submitted) => {
                                let short_contract_id =
                                    submitted.contract_id.chars().take(10).collect::<String>();

                                *submitted_for_publication.borrow_mut() = Some(submitted);

                                status_for_publication.set(format!(
                                    "Game contract {short_contract_id}… submitted; awaiting exact confirmation before challenge advertisement",
                                ));
                            }

                            Err(error) => {
                                /*
                                 * Keep the durable record and disabled state.
                                 * A later retry/recovery path must reuse that
                                 * exact signed evidence and game ID.
                                 */
                                pending_for_publication.set(true);
                                status_for_publication.set(
                                    "Challenge plan stored; game-contract publication requires retry"
                                        .to_owned(),
                                );
                                error_for_publication.set(Some(error));
                            }
                        }
                    });
                },
            )
        };

        let on_accept_incoming_challenge: Callback<SignedChallengeOffer> = {
            let local_player_id = local_player_id.clone();
            let authoritative_lobby_state = authoritative_lobby_state.clone();
            let freenet_api = freenet_api.clone();
            let outbound_pending = challenge_publication_pending.clone();
            let incoming_pending = incoming_acceptance_pending.clone();
            let incoming_status = incoming_acceptance_status.clone();
            let incoming_error = incoming_acceptance_error.clone();
            let pending_probe = pending_incoming_acceptance_probe.clone();
            let retrieved_read = retrieved_incoming_acceptance_read.clone();

            Callback::from(move |signed_offer: SignedChallengeOffer| {
                if *incoming_pending {
                    return;
                }

                let prepared = (|| {
                    if *outbound_pending {
                        return Err("An outbound challenge workflow is already pending.".to_owned());
                    }

                    let local_player_id = (*local_player_id)
                        .ok_or_else(|| "Local identity is not ready yet.".to_owned())?;

                    let now_unix_seconds = local_observation_unix_seconds()?;

                    let authoritative_state =
                        (*authoritative_lobby_state).as_ref().ok_or_else(|| {
                            "Verified authoritative lobby state is unavailable.".to_owned()
                        })?;

                    let mut exact_matches = authoritative_state
                        .challenges
                        .offers
                        .iter()
                        .filter(|entry| entry.offer == signed_offer);

                    let current_challenge = exact_matches.next().ok_or_else(|| {
                        "The selected signed challenge is no longer present \
                             in authoritative lobby state."
                            .to_owned()
                    })?;

                    if exact_matches.next().is_some() {
                        return Err("Authoritative lobby state contains ambiguous \
                             duplicate records for this signed challenge."
                            .to_owned());
                    }

                    if freenet_api.borrow().is_none() {
                        return Err("The Freenet connection is not available.".to_owned());
                    }

                    prepare_incoming_challenge_contract_probe(
                        current_challenge,
                        local_player_id,
                        now_unix_seconds,
                    )
                })();

                let probe = match prepared {
                    Ok(probe) => probe,

                    Err(error) => {
                        incoming_pending.set(false);
                        incoming_status
                            .set("Incoming acceptance probe could not be prepared".to_owned());
                        incoming_error.set(Some(error));
                        return;
                    }
                };

                let short_challenge_id = probe.signed_offer.body.challenge_id[..5]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();

                let contract_id = probe.contract_id.clone();

                /*
                 * Arm before the asynchronous send. Even an immediate response
                 * therefore has its exact unsigned originating evidence.
                 */
                retrieved_read.borrow_mut().take();
                *pending_probe.borrow_mut() = Some(probe);

                incoming_pending.set(true);
                incoming_error.set(None);
                incoming_status.set(format!(
                    "Challenge {short_challenge_id}… remains unsigned; \
                     requesting its exact game contract",
                ));

                let api = freenet_api.clone();
                let pending_probe_for_request = pending_probe.clone();
                let pending_for_request = incoming_pending.clone();
                let status_for_request = incoming_status.clone();
                let error_for_request = incoming_error.clone();

                wasm_bindgen_futures::spawn_local(async move {
                    let result = {
                        let mut api = api.borrow_mut();

                        match api.as_mut() {
                            Some(api) => request_contract(api, &contract_id).await,

                            None => Err("Freenet connection closed before the incoming \
                                 game-contract request."
                                .to_owned()),
                        }
                    };

                    if let Err(error) = result {
                        let should_clear = pending_probe_for_request
                            .borrow()
                            .as_ref()
                            .is_some_and(|probe| probe.contract_id == contract_id);

                        if should_clear {
                            pending_probe_for_request.borrow_mut().take();
                            pending_for_request.set(false);
                            status_for_request.set(
                                "Incoming game-contract request failed; \
                                 no acceptance was created"
                                    .to_owned(),
                            );
                            error_for_request.set(Some(error));
                        }
                    }
                });
            })
        };

        /*
         * Project opponents only from a complete, independently verified
         * authoritative lobby state. Local intent is never mixed into this
         * network-derived view.
         */
        let lobby_now = local_observation_unix_seconds();

        let available_players = match (
            *local_player_id,
            (*authoritative_lobby_state).as_ref(),
            lobby_now.as_ref(),
        ) {
            (Some(player_id), Some(state), Ok(now_unix_seconds)) => {
                let announcements = state
                    .lobby
                    .0
                    .players
                    .iter()
                    .flat_map(|player| player.records.iter().cloned())
                    .collect::<Vec<_>>();

                project_available_players(player_id, &announcements, *now_unix_seconds)
            }

            _ => Vec::new(),
        };

        /*
         * Incoming challenges are projected independently from the same
         * complete verified lobby state. Only live, unresolved, exactly
         * addressed signed offers survive this pure projection.
         */
        let incoming_challenges = match (
            *local_player_id,
            (*authoritative_lobby_state).as_ref(),
            lobby_now.as_ref(),
        ) {
            (Some(player_id), Some(state), Ok(now_unix_seconds)) => {
                project_incoming_challenges(player_id, &state.challenges.offers, *now_unix_seconds)
            }

            _ => Vec::new(),
        };

        /*
         * Accepted games are projected from the complete verified lobby state
         * without consulting the browser clock. This is a read-only runtime
         * candidate set; it does not yet select a game, retarget transport, or
         * replace the fixed test contract.
         */
        let accepted_games = match (*local_player_id, (*authoritative_lobby_state).as_ref()) {
            (Some(player_id), Some(state)) => {
                project_accepted_games(player_id, &state.challenges.offers)
            }

            _ => Ok(Vec::new()),
        };

        let lobby_network_detail = match lobby_now.as_ref() {
            Ok(_) => format!(
                "{} · Subscription: {} · {} · Expiry view uses this browser's clock",
                lobby_contract_status.label(),
                lobby_subscription_status.label(),
                lobby_contract_status.detail(),
            ),

            Err(error) => format!(
                "{} · Subscription: {} · Projection unavailable: {}",
                lobby_contract_status.label(),
                lobby_subscription_status.label(),
                error,
            ),
        };

        let lobby_empty_message = match (&*lobby_contract_status, lobby_now.as_ref()) {
            (LobbyContractStatus::Retrieved { .. }, Ok(_)) => {
                "No opponents are currently considered available by this browser."
            }

            (LobbyContractStatus::Failed(_), _) => {
                "Verified lobby players are currently unavailable."
            }

            (_, Err(_)) => "Available players cannot be projected without a valid browser clock.",

            _ => "No verified Freenet presence records loaded yet.",
        };

        let incoming_challenge_empty_message = match (&*lobby_contract_status, lobby_now.as_ref()) {
            (LobbyContractStatus::Retrieved { .. }, Ok(_)) => {
                "No live verified challenges are addressed to this identity."
            }

            (LobbyContractStatus::Failed(_), _) => {
                "Verified incoming challenges are currently unavailable."
            }

            (_, Err(_)) => "Incoming challenges cannot be projected without a valid browser clock.",

            _ => "No verified Freenet challenge records loaded yet.",
        };

        let lobby_identity_text = match *local_player_id {
            Some(player_id) => {
                let full = format_player_id(&player_id);
                format!("{}…{}", &full[..8], &full[full.len() - 8..])
            }

            None => "Identity unavailable".to_owned(),
        };

        let lobby_availability_label = if *lobby_presence_submission_pending {
            "Publishing"
        } else if *lobby_available {
            "Available"
        } else {
            "Unavailable"
        };

        let lobby_availability_action = if *lobby_presence_submission_pending {
            "Publishing presence…"
        } else if *lobby_available {
            "Go unavailable"
        } else {
            "Go available"
        };

        let lobby_profile_controls_disabled =
            (*local_player_id).is_none() || *lobby_available || *lobby_presence_submission_pending;

        let lobby_availability_disabled = (*local_player_id).is_none()
            || *lobby_presence_submission_pending
            || latest_lobby_contract_key.borrow().is_none();

        let challenge_controls_disabled = *challenge_publication_pending
            || (*local_player_id).is_none()
            || latest_lobby_contract_key.borrow().is_none()
            || freenet_api.borrow().is_none();

        let incoming_acceptance_controls_disabled = *incoming_acceptance_pending
            || *challenge_publication_pending
            || (*local_player_id).is_none()
            || (*authoritative_lobby_state).is_none()
            || freenet_api.borrow().is_none();

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

                <section
                    class="lobby-strip"
                    aria-label="Multiplayer lobby"
                >
                    <section
                        class="panel lobby-panel lobby-profile-panel"
                        aria-labelledby="lobby-profile-heading"
                    >
                        <div class="panel-heading-row">
                            <h2 id="lobby-profile-heading">
                                { "Lobby profile" }
                            </h2>

                            <span class="lobby-identity">
                                { lobby_identity_text }
                            </span>
                        </div>

                        <label
                            class="lobby-field-label"
                            for="lobby-display-name"
                        >
                            { "Public display name" }
                        </label>

                        <input
                            id="lobby-display-name"
                            class="lobby-name-input"
                            type="text"
                            autocomplete="off"
                            spellcheck="false"
                            value={(*lobby_display_name).clone()}
                            oninput={on_lobby_name_input}
                            disabled={lobby_profile_controls_disabled}
                        />

                        <div class="lobby-inline-actions">
                            <button
                                type="button"
                                onclick={on_save_lobby_name}
                                disabled={lobby_profile_controls_disabled}
                            >
                                { "Save name" }
                            </button>

                            <span class="lobby-profile-status">
                                { (*lobby_profile_status).clone() }
                            </span>
                        </div>

                        {
                            lobby_profile_error.as_ref().map_or_else(
                                || html! {},
                                |error| html! {
                                    <p
                                        class="interface-error"
                                        role="alert"
                                    >
                                        { error }
                                    </p>
                                },
                            )
                        }
                    </section>

                    <section
                        class="panel lobby-panel lobby-availability-panel"
                        aria-labelledby="lobby-availability-heading"
                    >
                        <div class="panel-heading-row">
                            <h2 id="lobby-availability-heading">
                                { "Availability" }
                            </h2>

                            <span
                                class={classes!(
                                    "availability-indicator",
                                    (*lobby_available).then_some("available"),
                                )}
                            >
                                { lobby_availability_label }
                            </span>
                        </div>

                        <p class="panel-note">
                            {
                                "This control publishes signed presence to Freenet. Revisions order updates; this browser's clock only bounds the ten-minute discovery lease."
                            }
                        </p>

                        <button
                            type="button"
                            class={classes!(
                                "availability-control",
                                (*lobby_available).then_some("selected"),
                            )}
                            aria-pressed={(*lobby_available).to_string()}
                            onclick={on_toggle_lobby_availability}
                            disabled={lobby_availability_disabled}
                        >
                            { lobby_availability_action }
                        </button>
                    </section>

                    <section
                        class="panel lobby-panel lobby-players-panel"
                        aria-labelledby="available-players-heading"
                    >
                        <div class="panel-heading-row">
                            <h2 id="available-players-heading">
                                { "Available players" }
                            </h2>

                            <span class="history-count">
                                { available_players.len() }
                            </span>
                        </div>

                        <p class="panel-note" role="status">
                            { lobby_network_detail }
                        </p>

                        <p
                            class="challenge-publication-status"
                            role="status"
                        >
                            { (*challenge_publication_status).clone() }
                        </p>

                        {
                            challenge_publication_error.as_ref().map_or_else(
                                || html! {},
                                |error| html! {
                                    <p
                                        class="interface-error"
                                        role="alert"
                                    >
                                        { error }
                                    </p>
                                },
                            )
                        }

                        {
                            if available_players.is_empty() {
                                html! {
                                    <p class="lobby-empty-state">
                                        { lobby_empty_message }
                                    </p>
                                }
                            } else {
                                html! {
                                    <ul class="available-player-list">
                                        {
                                            for available_players.iter().map(
                                                |player| {
                                                    let challenge_target = (
                                                        player.player_id,
                                                        player.display_name.clone(),
                                                        player.expires_at_unix_seconds,
                                                    );

                                                    let on_challenge = {
                                                        let on_challenge_player =
                                                            on_challenge_player.clone();

                                                        Callback::from(move |_| {
                                                            on_challenge_player.emit(
                                                                challenge_target.clone(),
                                                            );
                                                        })
                                                    };

                                                    html! {
                                                        <li>
                                                            <strong>
                                                                {
                                                                    player
                                                                        .display_name
                                                                        .clone()
                                                                }
                                                            </strong>

                                                            <span>
                                                                {
                                                                    format_player_id(
                                                                        &player.player_id
                                                                    )
                                                                }
                                                            </span>

                                                            <button
                                                                type="button"
                                                                class="challenge-player-button"
                                                                onclick={on_challenge}
                                                                disabled={
                                                                    challenge_controls_disabled
                                                                }
                                                            >
                                                                {
                                                                    if *challenge_publication_pending {
                                                                        "Challenge pending"
                                                                    } else {
                                                                        "Challenge"
                                                                    }
                                                                }
                                                            </button>
                                                        </li>
                                                    }
                                                }
                                            )
                                        }
                                    </ul>
                                }
                            }
                        }
                    </section>

                    <section
                        class="panel lobby-panel lobby-incoming-panel"
                        aria-labelledby="incoming-challenges-heading"
                    >
                        <div class="panel-heading-row">
                            <h2 id="incoming-challenges-heading">
                                { "Incoming challenges" }
                            </h2>

                            <span class="history-count">
                                { incoming_challenges.len() }
                            </span>
                        </div>

                        <p class="panel-note" role="status">
                            {
                                (*incoming_acceptance_status).clone()
                            }
                        </p>

                        {
                            match &*incoming_acceptance_error {
                                Some(error) => html! {
                                    <p class="interface-error" role="alert">
                                        { error.clone() }
                                    </p>
                                },

                                None => html! {},
                            }
                        }

                        {
                            if incoming_challenges.is_empty() {
                                html! {
                                    <p class="lobby-empty-state">
                                        {
                                            incoming_challenge_empty_message
                                        }
                                    </p>
                                }
                            } else {
                                html! {
                                    <ul class="incoming-challenge-list">
                                        {
                                            for incoming_challenges.iter().map(
                                                |challenge| {
                                                    let challenger_id =
                                                        format_player_id(
                                                            &challenge
                                                                .challenger_id
                                                        );

                                                    let challenge_id =
                                                        format_player_id(
                                                            &challenge
                                                                .challenge_id
                                                        );

                                                    let game_id =
                                                        format_player_id(
                                                            &challenge.game_id
                                                        );

                                                    let signed_offer =
                                                        challenge
                                                            .signed_offer
                                                            .clone();

                                                    let on_accept = {
                                                        let callback =
                                                            on_accept_incoming_challenge
                                                                .clone();

                                                        Callback::from(
                                                            move |_| {
                                                                callback.emit(
                                                                    signed_offer
                                                                        .clone(),
                                                                );
                                                            },
                                                        )
                                                    };

                                                    html! {
                                                        <li>
                                                            <div
                                                                class="incoming-challenge-heading"
                                                            >
                                                                <strong>
                                                                    {
                                                                        challenge
                                                                            .challenger_display_name
                                                                            .clone()
                                                                    }
                                                                </strong>

                                                                <span>
                                                                    {
                                                                        format!(
                                                                            "Match length: {}",
                                                                            challenge.match_length,
                                                                        )
                                                                    }
                                                                </span>
                                                            </div>

                                                            <dl
                                                                class="incoming-challenge-details"
                                                            >
                                                                <div>
                                                                    <dt>
                                                                        { "Challenger ID" }
                                                                    </dt>
                                                                    <dd>
                                                                        { challenger_id }
                                                                    </dd>
                                                                </div>

                                                                <div>
                                                                    <dt>
                                                                        { "Challenge ID" }
                                                                    </dt>
                                                                    <dd>
                                                                        { challenge_id }
                                                                    </dd>
                                                                </div>

                                                                <div>
                                                                    <dt>
                                                                        { "Game ID" }
                                                                    </dt>
                                                                    <dd>
                                                                        { game_id }
                                                                    </dd>
                                                                </div>

                                                                <div>
                                                                    <dt>
                                                                        { "Signed expiry" }
                                                                    </dt>
                                                                    <dd>
                                                                        {
                                                                            format!(
                                                                                "{} Unix seconds",
                                                                                challenge
                                                                                    .expires_at_unix_seconds,
                                                                            )
                                                                        }
                                                                    </dd>
                                                                </div>
                                                            </dl>

                                                            <button
                                                                type="button"
                                                                class="challenge-player-button"
                                                                onclick={on_accept}
                                                                disabled={
                                                                    incoming_acceptance_controls_disabled
                                                                }
                                                            >
                                                                {
                                                                    if *incoming_acceptance_pending {
                                                                        "Acceptance pending"
                                                                    } else {
                                                                        "Accept challenge"
                                                                    }
                                                                }
                                                            </button>
                                                        </li>
                                                    }
                                                }
                                            )
                                        }
                                    </ul>
                                }
                            }
                        }
                    </section>
                </section>

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
                                    <dt>{ "Accepted games" }</dt>
                                    <dd>
                                        {
                                            match &accepted_games {
                                                Ok(games) => {
                                                    format!("{} verified", games.len())
                                                }
                                                Err(error) => {
                                                    format!("Projection unavailable: {error}")
                                                }
                                            }
                                        }
                                    </dd>
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
