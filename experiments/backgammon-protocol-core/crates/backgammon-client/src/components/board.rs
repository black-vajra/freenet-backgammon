use backgammon_core::{MoveSource, MoveTarget, Player};
use yew::prelude::*;

use crate::components::checker::Checker;
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

fn bar_checker_y(player: Player, index: u8) -> f32 {
    let offset = f32::from(index) * 42.0;

    match player {
        Player::Black => 112.0 + offset,
        Player::White => 688.0 - offset,
    }
}

fn borne_off_checker_y(player: Player, index: u8) -> f32 {
    let offset = f32::from(index) * 15.0;

    match player {
        Player::Black => 137.0 + offset,
        Player::White => 683.0 - offset,
    }
}

fn bar_checkers(
    player: Player,
    count: u8,
    selectable: bool,
    selected: bool,
    on_source: &Callback<MoveSource>,
) -> Html {
    let visible_count = count.min(5);

    html! {
        <>
            {
                for (0..visible_count).map(|index| {
                    let onclick = if selectable {
                        let callback = on_source.clone();

                        Callback::from(move |_| {
                            callback.emit(MoveSource::Bar);
                        })
                    } else {
                        Callback::noop()
                    };

                    html! {
                        <Checker
                            player={player}
                            x={600.0}
                            y={bar_checker_y(player, index)}
                            label={format!(
                                "{} checker on the bar",
                                match player {
                                    Player::White => "White",
                                    Player::Black => "Black",
                                }
                            )}
                            selectable={selectable}
                            selected={selected}
                            onclick={onclick}
                        />
                    }
                })
            }

            {
                if count > visible_count {
                    let badge_y = match player {
                        Player::Black => 112.0 + f32::from(visible_count) * 42.0,
                        Player::White => 688.0 - f32::from(visible_count) * 42.0,
                    };

                    html! {
                        <g class="bar-overflow-badge" pointer-events="none">
                            <circle
                                cx="600"
                                cy={badge_y.to_string()}
                                r="18"
                            />

                            <text
                                x="600"
                                y={(badge_y + 6.0).to_string()}
                                text-anchor="middle"
                            >
                                { format!("+{}", count - visible_count) }
                            </text>
                        </g>
                    }
                } else {
                    html! {}
                }
            }
        </>
    }
}

fn borne_off_checkers(player: Player, count: u8) -> Html {
    html! {
        <>
            {
                for (0..count).map(|index| {
                    let player_class = match player {
                        Player::White => "white",
                        Player::Black => "black",
                    };

                    html! {
                        <rect
                            class={classes!("borne-off-checker", player_class)}
                            x="1123"
                            y={borne_off_checker_y(player, index).to_string()}
                            width="34"
                            height="11"
                            rx="5.5"
                        >
                            <title>
                                {
                                    format!(
                                        "{} borne-off checker {}",
                                        match player {
                                            Player::White => "White",
                                            Player::Black => "Black",
                                        },
                                        index + 1
                                    )
                                }
                            </title>
                        </rect>
                    }
                })
            }
        </>
    }
}

#[function_component(Board)]
pub fn board(props: &BoardProps) -> Html {
    let bar_is_legal = props.legal_sources.contains(&MoveSource::Bar);
    let bar_is_selected = props.selected_source == Some(MoveSource::Bar);
    let bear_off_is_legal = props.legal_destinations.contains(&MoveTarget::BearOff);

    let black_bar_selectable = bar_is_legal && props.board.active_player == Player::Black;

    let white_bar_selectable = bar_is_legal && props.board.active_player == Player::White;

    let black_bar_selected = bar_is_selected && props.board.active_player == Player::Black;

    let white_bar_selected = bar_is_selected && props.board.active_player == Player::White;

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
            onmousedown={Callback::from(|event: MouseEvent| {
                event.prevent_default();
            })}
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

            <g class="bar-area">
                <rect
                    x="555"
                    y="55"
                    width="90"
                    height="690"
                    fill="#321b0e"
                    stroke="#211008"
                    stroke-width="4"
                />

                <line
                    x1="565"
                    y1="400"
                    x2="635"
                    y2="400"
                    class="bar-divider"
                />

                <text
                    x="600"
                    y="78"
                    text-anchor="middle"
                    class="bar-player-label"
                    pointer-events="none"
                >
                    { "BLACK BAR" }
                </text>

                <text
                    x="600"
                    y="730"
                    text-anchor="middle"
                    class="bar-player-label"
                    pointer-events="none"
                >
                    { "WHITE BAR" }
                </text>

                {
                    bar_checkers(
                        Player::Black,
                        props.board.black_bar,
                        black_bar_selectable,
                        black_bar_selected,
                        &props.on_source,
                    )
                }

                {
                    bar_checkers(
                        Player::White,
                        props.board.white_bar,
                        white_bar_selectable,
                        white_bar_selected,
                        &props.on_source,
                    )
                }

                {
                    if props.board.black_bar == 0 {
                        html! {
                            <text
                                x="600"
                                y="122"
                                text-anchor="middle"
                                class="bar-empty-count"
                                pointer-events="none"
                            >
                                { "0" }
                            </text>
                        }
                    } else {
                        html! {}
                    }
                }

                {
                    if props.board.white_bar == 0 {
                        html! {
                            <text
                                x="600"
                                y="686"
                                text-anchor="middle"
                                class="bar-empty-count"
                                pointer-events="none"
                            >
                                { "0" }
                            </text>
                        }
                    } else {
                        html! {}
                    }
                }
            </g>

            <g
                class={classes!(
                    "bear-off-area",
                    "black-off-area",
                    black_bear_off_legal.then_some("destination-legal"),
                )}
                role={black_bear_off_legal.then_some("button")}
                aria-label="Black bear-off tray"
                onclick={
                    if black_bear_off_legal {
                        on_bear_off.clone()
                    } else {
                        Callback::noop()
                    }
                }
            >
                <rect
                    x="1114"
                    y="70"
                    width="52"
                    height="300"
                    rx="10"
                    class="bear-off-tray"
                />

                <text
                    x="1140"
                    y="94"
                    text-anchor="middle"
                    class="tray-label"
                    pointer-events="none"
                >
                    <tspan x="1140" dy="0">{ "BLACK" }</tspan>
                    <tspan x="1140" dy="14">{ "OFF" }</tspan>
                </text>

                {
                    borne_off_checkers(
                        Player::Black,
                        props.board.black_borne_off,
                    )
                }

                <g class="tray-count-badge" pointer-events="none">
                    <circle cx="1140" cy="345" r="16" />
                    <text x="1140" y="351" text-anchor="middle">
                        { props.board.black_borne_off }
                    </text>
                </g>
            </g>

            <g
                class={classes!(
                    "bear-off-area",
                    "white-off-area",
                    white_bear_off_legal.then_some("destination-legal"),
                )}
                role={white_bear_off_legal.then_some("button")}
                aria-label="White bear-off tray"
                onclick={
                    if white_bear_off_legal {
                        on_bear_off
                    } else {
                        Callback::noop()
                    }
                }
            >
                <rect
                    x="1114"
                    y="430"
                    width="52"
                    height="300"
                    rx="10"
                    class="bear-off-tray"
                />

                <text
                    x="1140"
                    y="708"
                    text-anchor="middle"
                    class="tray-label"
                    pointer-events="none"
                >
                    <tspan x="1140" dy="0">{ "WHITE" }</tspan>
                    <tspan x="1140" dy="14">{ "OFF" }</tspan>
                </text>

                {
                    borne_off_checkers(
                        Player::White,
                        props.board.white_borne_off,
                    )
                }

                <g class="tray-count-badge" pointer-events="none">
                    <circle cx="1140" cy="455" r="16" />
                    <text x="1140" y="461" text-anchor="middle">
                        { props.board.white_borne_off }
                    </text>
                </g>
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
    use super::{bar_checker_y, borne_off_checker_y, point_position};
    use backgammon_core::Player;

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

    #[test]
    fn bar_checkers_stack_away_from_each_end() {
        assert_eq!(bar_checker_y(Player::Black, 0), 112.0);
        assert_eq!(bar_checker_y(Player::Black, 1), 154.0);

        assert_eq!(bar_checker_y(Player::White, 0), 688.0);
        assert_eq!(bar_checker_y(Player::White, 1), 646.0);
    }

    #[test]
    fn borne_off_checkers_stack_inside_their_trays() {
        assert_eq!(borne_off_checker_y(Player::Black, 0), 137.0);
        assert_eq!(borne_off_checker_y(Player::Black, 14), 347.0);

        assert_eq!(borne_off_checker_y(Player::White, 0), 683.0);
        assert_eq!(borne_off_checker_y(Player::White, 14), 473.0);
    }
}
