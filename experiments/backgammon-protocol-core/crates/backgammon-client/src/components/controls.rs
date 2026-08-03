use yew::prelude::*;

#[function_component(GameControls)]
pub fn game_controls() -> Html {
    html! {
        <section class="panel controls-panel" aria-labelledby="controls-heading">
            <h2 id="controls-heading">{ "Game controls" }</h2>

            <div class="control-grid">
                <button type="button" class="primary-control" disabled=true>
                    { "Roll" }
                </button>

                <button type="button" disabled=true>
                    { "Resign" }
                </button>

                <button type="button" disabled=true>
                    { "Reconnect" }
                </button>

                <button type="button" disabled=true>
                    { "Leave" }
                </button>
            </div>

            <p class="panel-note">
                { "Controls activate in the local-play milestone." }
            </p>
        </section>
    }
}
