use backgammon_core::Player;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct CheckerProps {
    pub player: Player,
    pub x: f32,
    pub y: f32,
    pub label: String,
    pub selectable: bool,
    pub selected: bool,
    pub onclick: Callback<MouseEvent>,
}

#[function_component(Checker)]
pub fn checker(props: &CheckerProps) -> Html {
    let (fill, text_fill, symbol) = match props.player {
        Player::White => ("#f4ead7", "#2b1a12", "W"),
        Player::Black => ("#241713", "#f4ead7", "B"),
    };

    html! {
        <g
            class={classes!(
                "checker",
                props.selectable.then_some("checker-selectable"),
                props.selected.then_some("checker-selected"),
            )}
            role={props.selectable.then_some("button")}
            aria-label={props.label.clone()}
            onclick={props.onclick.clone()}
        >
            <title>{ props.label.clone() }</title>

            <circle
                class="checker-selection-ring"
                cx={props.x.to_string()}
                cy={props.y.to_string()}
                r="27"
            />

            <circle
                cx={props.x.to_string()}
                cy={props.y.to_string()}
                r="22"
                fill={fill}
                stroke="#0f0906"
                stroke-width="3"
            />

            <circle
                cx={props.x.to_string()}
                cy={props.y.to_string()}
                r="17"
                fill="none"
                stroke="rgba(255, 255, 255, 0.25)"
                stroke-width="1.5"
            />

            <text
                x={props.x.to_string()}
                y={(props.y + 6.0).to_string()}
                text-anchor="middle"
                font-size="16"
                font-weight="800"
                fill={text_fill}
                pointer-events="none"
            >
                { symbol }
            </text>
        </g>
    }
}
