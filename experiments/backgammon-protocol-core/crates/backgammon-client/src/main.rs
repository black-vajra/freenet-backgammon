#[cfg(target_arch = "wasm32")]
mod components;

pub mod controller;
pub mod projection;

#[cfg(target_arch = "wasm32")]
mod browser {
    use backgammon_core::{Dice, GameStatus, MoveSource, MoveTarget, Player, TurnPhase};
    use yew::prelude::*;

    use crate::components::board::Board;
    use crate::components::controls::GameControls;
    use crate::components::dice::DiceDisplay;
    use crate::components::history::MoveHistory;
    use crate::components::player_panel::PlayerPanel;
    use crate::controller::{LocalGameController, LocalGameOutcome};
    use crate::projection::BoardView;

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

    #[function_component(App)]
    fn app() -> Html {
        let controller = use_state(LocalGameController::new);
        let interface_error = use_state(|| None::<String>);

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
            let controller = controller.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |_| {
                let mut next = (*controller).clone();

                match next.resign() {
                    Ok(()) => {
                        interface_error.set(None);
                        controller.set(next);
                    }
                    Err(error) => {
                        interface_error
                            .set(Some(format!("The game could not be resigned: {error:?}")));
                    }
                }
            })
        };

        let on_new_game = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |_| {
                let mut next = (*controller).clone();
                next.new_game();

                interface_error.set(None);
                controller.set(next);
            })
        };

        let on_leave = {
            let controller = controller.clone();
            let interface_error = interface_error.clone();

            Callback::from(move |_| {
                let mut next = (*controller).clone();
                next.leave_table();

                interface_error.set(None);
                controller.set(next);
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

                    <div class="connection-badge" role="status">
                        <span class="connection-dot" aria-hidden="true"></span>
                        {
                            if left_table {
                                "Table left"
                            } else {
                                "Local mode"
                            }
                        }
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
                            on_roll={on_roll}
                            on_pass={on_pass}
                            on_resign={on_resign}
                            on_new_game={on_new_game.clone()}
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

                            <dl class="status-list">
                                <div>
                                    <dt>{ "Mode" }</dt>
                                    <dd>{ "Local" }</dd>
                                </div>

                                <div>
                                    <dt>{ "Opponent" }</dt>
                                    <dd>{ "Same device" }</dd>
                                </div>

                                <div>
                                    <dt>{ "State" }</dt>
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
