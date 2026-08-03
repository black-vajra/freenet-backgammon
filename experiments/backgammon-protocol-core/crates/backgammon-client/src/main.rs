#[cfg(target_arch = "wasm32")]
mod components;

pub mod controller;
pub mod projection;

#[cfg(target_arch = "wasm32")]
mod browser {
    use backgammon_core::{Dice, GameStatus, Player, TurnPhase};
    use yew::prelude::*;

    use crate::components::board::Board;
    use crate::components::controls::GameControls;
    use crate::components::dice::DiceDisplay;
    use crate::components::history::MoveHistory;
    use crate::components::player_panel::PlayerPanel;
    use crate::controller::LocalGameController;
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

    #[function_component(App)]
    fn app() -> Html {
        let controller = use_state(LocalGameController::new);
        let interface_error = use_state(|| None::<String>);

        let board = BoardView::from(controller.visible_state());

        let can_roll = matches!(controller.state().status, GameStatus::InProgress)
            && controller.state().turn_phase == TurnPhase::AwaitingRoll;

        let active_name = player_name(board.active_player);

        let turn_text = match board.turn_phase {
            TurnPhase::AwaitingRoll => format!("{active_name} to roll"),
            TurnPhase::Moving => format!("{active_name} is moving"),
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

        html! {
            <main class="app-shell">
                <header class="app-header">
                    <div>
                        <p class="mode-label">{ "LOCAL TWO-PLAYER MODE" }</p>
                        <h1>{ "Freenet Backgammon" }</h1>
                    </div>

                    <div class="connection-badge" role="status">
                        <span class="connection-dot" aria-hidden="true"></span>
                        { "Local mode" }
                    </div>
                </header>

                <section class="game-layout">
                    <aside class="left-rail">
                        <PlayerPanel
                            player={Player::Black}
                            name={"Player Two".to_owned()}
                            score={0}
                            active={board.active_player == Player::Black}
                            bar={board.black_bar}
                            borne_off={board.black_borne_off}
                        />

                        <PlayerPanel
                            player={Player::White}
                            name={"Player One".to_owned()}
                            score={0}
                            active={board.active_player == Player::White}
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
                            on_roll={on_roll}
                        />
                    </aside>

                    <section class="board-stage" aria-label="Game board">
                        <Board board={board} />
                    </section>

                    <aside class="right-rail">
                        <MoveHistory />

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
                                            if can_roll {
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
