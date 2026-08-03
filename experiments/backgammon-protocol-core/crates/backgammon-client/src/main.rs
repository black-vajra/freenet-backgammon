#[cfg(target_arch = "wasm32")]
mod browser {
    use backgammon_core::GameState;
    use yew::prelude::*;

    #[function_component(App)]
    fn app() -> Html {
        // This is deliberately a real rules-engine value rather than a
        // framework-only Hello World. Successful browser execution proves
        // that the transport-independent core links into the WASM client.
        let _initial_state = GameState::standard_start();

        html! {
            <main class="client-shell">
                <section class="proof-card" aria-labelledby="application-title">
                    <p class="mode-label">{ "LOCAL TWO-PLAYER MODE" }</p>

                    <h1 id="application-title">
                        { "Freenet Backgammon" }
                    </h1>

                    <p class="summary">
                        { "The graphical-client workspace is running in the browser." }
                    </p>

                    <p class="core-proof">
                        <span aria-hidden="true">{ "✓" }</span>
                        { " backgammon-core constructed the standard starting state." }
                    </p>

                    <p class="next-step">
                        { "Next milestone: render the 24-point board from GameState." }
                    </p>
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
