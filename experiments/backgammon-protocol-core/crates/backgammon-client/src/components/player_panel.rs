use backgammon_core::Player;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct PlayerPanelProps {
    pub player: Player,
    pub name: String,
    pub score: u8,
    pub active: bool,
    pub bar: u8,
    pub borne_off: u8,
}

#[function_component(PlayerPanel)]
pub fn player_panel(props: &PlayerPanelProps) -> Html {
    let player_class = match props.player {
        Player::White => "white",
        Player::Black => "black",
    };

    let checker_label = match props.player {
        Player::White => "W",
        Player::Black => "B",
    };

    let active_text = if props.active {
        "Current turn"
    } else {
        "Waiting"
    };

    html! {
        <section
            class={classes!("panel", "player-panel", player_class, props.active.then_some("active"))}
            aria-label={format!("{} player panel", props.name)}
        >
            <div class="player-heading">
                <span
                    class={classes!("player-checker", player_class)}
                    aria-label={format!("{checker_label} checker")}
                >
                    { checker_label }
                </span>

                <div>
                    <h2>{ &props.name }</h2>
                    <p class="player-state">{ active_text }</p>
                </div>

                <div class="score-block" aria-label={format!("Score: {}", props.score)}>
                    <span>{ "Score" }</span>
                    <strong>{ props.score }</strong>
                </div>
            </div>

            <dl class="player-counters">
                <div>
                    <dt>{ "Bar" }</dt>
                    <dd>{ props.bar }</dd>
                </div>

                <div>
                    <dt>{ "Borne off" }</dt>
                    <dd>{ props.borne_off }</dd>
                </div>
            </dl>
        </section>
    }
}
