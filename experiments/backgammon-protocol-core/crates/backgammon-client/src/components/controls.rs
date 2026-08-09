use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct GameControlsProps {
    pub can_roll: bool,
    pub can_pass: bool,
    pub can_resign: bool,
    pub can_leave: bool,
    pub can_reconnect: bool,
    pub status_note: String,
    pub on_roll: Callback<MouseEvent>,
    pub on_pass: Callback<MouseEvent>,
    pub on_resign: Callback<MouseEvent>,
    pub on_new_game: Callback<MouseEvent>,
    pub on_reconnect: Callback<MouseEvent>,
    pub on_leave: Callback<MouseEvent>,
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

                <button
                    type="button"
                    class="pass-control"
                    disabled={!props.can_pass}
                    onclick={props.on_pass.clone()}
                >
                    { "Pass turn" }
                </button>

                <button
                    type="button"
                    disabled={!props.can_resign}
                    onclick={props.on_resign.clone()}
                >
                    { "Resign" }
                </button>

                <button
                    type="button"
                    class="new-game-control"
                    onclick={props.on_new_game.clone()}
                >
                    { "New game" }
                </button>

                <button
                    type="button"
                    class="reconnect-control"
                    disabled={!props.can_reconnect}
                    onclick={props.on_reconnect.clone()}
                >
                    { "Reconnect" }
                </button>

                <button
                    type="button"
                    disabled={!props.can_leave}
                    onclick={props.on_leave.clone()}
                >
                    { "Leave" }
                </button>
            </div>

            <p class="panel-note">
                { props.status_note.clone() }
            </p>

            <p class="control-footnote">
                { "Reconnect currently restores the local Freenet node connection." }
            </p>
        </section>
    }
}
