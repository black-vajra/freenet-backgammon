#[cfg(target_arch = "wasm32")]
mod components;

pub mod projection;

#[cfg(target_arch = "wasm32")]
mod browser {
    use backgammon_core::GameState;
    use yew::prelude::*;

    use crate::components::board::Board;
    use crate::projection::BoardView;

    #[function_component(App)]
    fn app() -> Html {
        let state = GameState::standard_start();
        let board = BoardView::from(&state);

        html! {
            <main class="client-shell">
                <h1>
                    { "Freenet Backgammon" }
                </h1>

                <Board board={board} />
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
