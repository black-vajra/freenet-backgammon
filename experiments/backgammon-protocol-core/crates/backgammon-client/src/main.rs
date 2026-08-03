#[cfg(target_arch = "wasm32")]
mod components;

pub mod projection;

#[cfg(target_arch = "wasm32")]
mod browser {
    use backgammon_core::{GameState, Player, TurnPhase};
    use yew::prelude::*;

    use crate::components::board::Board;
    use crate::components::controls::GameControls;
    use crate::components::dice::DiceDisplay;
    use crate::components::history::MoveHistory;
    use crate::components::player_panel::PlayerPanel;
    use crate::projection::BoardView;

    #[function_component(App)]
    fn app() -> Html {
        let state = GameState::standard_start();
        let board = BoardView::from(&state);

        let turn_text = match board.turn_phase {
            TurnPhase::AwaitingRoll => "White to roll",
            TurnPhase::Moving => "White is moving",
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
                            <p class="panel-note">{ "Game 1 · Single game" }</p>
                        </section>

                        <DiceDisplay dice={board.dice} />
                        <GameControls />
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
                                    <dd>{ "Ready" }</dd>
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
