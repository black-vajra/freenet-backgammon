use yew::prelude::*;

#[function_component(MoveHistory)]
pub fn move_history() -> Html {
    html! {
        <section class="panel history-panel" aria-labelledby="history-heading">
            <div class="panel-heading-row">
                <h2 id="history-heading">{ "Move history" }</h2>
                <span class="history-count">{ "0" }</span>
            </div>

            <ol class="move-history">
                <li class="history-placeholder">
                    { "No moves yet. White begins." }
                </li>
            </ol>
        </section>
    }
}
