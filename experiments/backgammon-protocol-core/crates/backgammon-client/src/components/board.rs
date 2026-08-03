use backgammon_core::{MoveSource, MoveTarget, Player};
use yew::prelude::*;

use crate::components::point::Point;
use crate::projection::BoardView;

#[derive(Properties, PartialEq)]
pub struct BoardProps {
    pub board: BoardView,
    pub legal_sources: Vec<MoveSource>,
    pub selected_source: Option<MoveSource>,
    pub legal_destinations: Vec<MoveTarget>,
    pub on_source: Callback<MoveSource>,
    pub on_destination: Callback<MoveTarget>,
}

fn point_position(index: usize) -> (f32, bool) {
    const LEFT: [f32; 6] = [110.0, 180.0, 250.0, 320.0, 390.0, 460.0];
    const RIGHT: [f32; 6] = [740.0, 810.0, 880.0, 950.0, 1020.0, 1090.0];

    match index {
        0..=5 => (RIGHT[5 - index], false),
        6..=11 => (LEFT[11 - index], false),
        12..=17 => (LEFT[index - 12], true),
        18..=23 => (RIGHT[index - 18], true),
        _ => unreachable!("backgammon point index must be between 0 and 23"),
    }
}

#[function_component(Board)]
pub fn board(props: &BoardProps) -> Html {
    let bar_is_legal = props.legal_sources.contains(&MoveSource::Bar);
    let bar_is_selected = props.selected_source == Some(MoveSource::Bar);
    let bear_off_is_legal = props.legal_destinations.contains(&MoveTarget::BearOff);

    let on_bar = {
        let callback = props.on_source.clone();

        Callback::from(move |_| {
            callback.emit(MoveSource::Bar);
        })
    };

    let bar_click = if bar_is_legal {
        on_bar
    } else {
        Callback::noop()
    };

    let on_bear_off = {
        let callback = props.on_destination.clone();

        Callback::from(move |_| {
            callback.emit(MoveTarget::BearOff);
        })
    };

    let white_bear_off_legal = bear_off_is_legal && props.board.active_player == Player::White;

    let black_bear_off_legal = bear_off_is_legal && props.board.active_player == Player::Black;

    html! {
        <svg
            class="backgammon-board"
            viewBox="0 0 1200 800"
            role="img"
            aria-label="Interactive backgammon board"
        >
            <title>{ "Interactive backgammon board" }</title>

            <rect
                x="20"
                y="20"
                width="1160"
                height="760"
                rx="30"
                fill="#6f4927"
                stroke="#3b2114"
                stroke-width="12"
            />

            <rect
                x="55"
                y="55"
                width="1090"
                height="690"
                rx="12"
                fill="#80552d"
                stroke="#3b2114"
                stroke-width="4"
            />

            <g
                class={classes!(
                    "bar-area",
                    bar_is_legal.then_some("bar-selectable"),
                    bar_is_selected.then_some("bar-selected"),
                )}
                onclick={bar_click}
            >
                <rect
                    x="555"
                    y="55"
                    width="90"
                    height="690"
                    fill="#321b0e"
                    stroke="#211008"
                    stroke-width="4"
                />

                <text
                    x="600"
                    y="360"
                    text-anchor="middle"
                    class="board-label"
                    pointer-events="none"
                >
                    { "BLACK BAR" }
                </text>

                <text
                    x="600"
                    y="392"
                    text-anchor="middle"
                    class="board-count"
                    pointer-events="none"
                >
                    { props.board.black_bar }
                </text>

                <text
                    x="600"
                    y="435"
                    text-anchor="middle"
                    class="board-label"
                    pointer-events="none"
                >
                    { "WHITE BAR" }
                </text>

                <text
                    x="600"
                    y="467"
                    text-anchor="middle"
                    class="board-count"
                    pointer-events="none"
                >
                    { props.board.white_bar }
                </text>
            </g>

            <g
                class={classes!(
                    "bear-off-area",
                    black_bear_off_legal.then_some("destination-legal"),
                )}
                onclick={
                    if black_bear_off_legal {
                        on_bear_off.clone()
                    } else {
                        Callback::noop()
                    }
                }
            >
                <rect
                    x="1132"
                    y="70"
                    width="34"
                    height="300"
                    rx="10"
                    class="bear-off-tray"
                />

                <text
                    x="1149"
                    y="105"
                    text-anchor="middle"
                    class="tray-label"
                    pointer-events="none"
                >
                    { "B" }
                </text>

                <text
                    x="1149"
                    y="220"
                    text-anchor="middle"
                    class="tray-count"
                    pointer-events="none"
                >
                    { props.board.black_borne_off }
                </text>
            </g>

            <g
                class={classes!(
                    "bear-off-area",
                    white_bear_off_legal.then_some("destination-legal"),
                )}
                onclick={
                    if white_bear_off_legal {
                        on_bear_off
                    } else {
                        Callback::noop()
                    }
                }
            >
                <rect
                    x="1132"
                    y="430"
                    width="34"
                    height="300"
                    rx="10"
                    class="bear-off-tray"
                />

                <text
                    x="1149"
                    y="465"
                    text-anchor="middle"
                    class="tray-label"
                    pointer-events="none"
                >
                    { "W" }
                </text>

                <text
                    x="1149"
                    y="580"
                    text-anchor="middle"
                    class="tray-count"
                    pointer-events="none"
                >
                    { props.board.white_borne_off }
                </text>
            </g>

            {
                for props.board.points.iter().map(|point| {
                    let (x, top) = point_position(point.index);

                    let point_index = u8::try_from(point.index)
                        .expect("projected point index must fit in u8");

                    let source = MoveSource::Point(point_index);
                    let destination = MoveTarget::Point(point_index);

                    html! {
                        <Point
                            point={*point}
                            x={x}
                            top={top}
                            source_selectable={props.legal_sources.contains(&source)}
                            source_selected={props.selected_source == Some(source)}
                            destination_legal={props.legal_destinations.contains(&destination)}
                            on_source={props.on_source.clone()}
                            on_destination={props.on_destination.clone()}
                        />
                    }
                })
            }
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::point_position;

    #[test]
    fn all_points_have_distinct_quadrant_positions() {
        let positions: Vec<(i32, bool)> = (0..24)
            .map(|index| {
                let (x, top) = point_position(index);
                (x as i32, top)
            })
            .collect();

        for left in 0..positions.len() {
            for right in left + 1..positions.len() {
                assert_ne!(
                    positions[left],
                    positions[right],
                    "points {} and {} overlap",
                    left + 1,
                    right + 1
                );
            }
        }
    }

    #[test]
    fn point_numbering_follows_conventional_orientation() {
        assert_eq!(point_position(0), (1090.0, false));
        assert_eq!(point_position(5), (740.0, false));
        assert_eq!(point_position(6), (460.0, false));
        assert_eq!(point_position(11), (110.0, false));

        assert_eq!(point_position(12), (110.0, true));
        assert_eq!(point_position(17), (460.0, true));
        assert_eq!(point_position(18), (740.0, true));
        assert_eq!(point_position(23), (1090.0, true));
    }
}
