use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct GameControlsProps {
    pub can_roll: bool,
    pub on_roll: Callback<MouseEvent>,
}

#[function_component(GameControls)]
pub fn game_controls(props: &GameControlsProps) -> Html {
    html! {
        <section class="panel controls-panel" aria-labelledby="controls-heading">
            <h2 id="controls-heading">{ "Game controls" }</h2>

            <div class="control-grid">
                <button
                    type="button"
                    class="primary-control"
                    disabled={!props.can_roll}
                    onclick={props.on_roll.clone()}
                >
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
                {
                    if props.can_roll {
                        "Roll to begin the turn."
                    } else {
                        "Complete the current turn before rolling again."
                    }
                }
            </p>
        </section>
    }
}
