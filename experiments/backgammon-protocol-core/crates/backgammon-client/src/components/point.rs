use backgammon_core::{MoveSource, MoveTarget, Player};
use yew::prelude::*;

use crate::components::checker::Checker;
use crate::projection::PointView;

#[derive(Properties, PartialEq)]
pub struct PointProps {
    pub point: PointView,
    pub x: f32,
    pub top: bool,
    pub source_selectable: bool,
    pub source_selected: bool,
    pub destination_legal: bool,
    pub on_source: Callback<MoveSource>,
    pub on_destination: Callback<MoveTarget>,
}

#[function_component(Point)]
pub fn point(props: &PointProps) -> Html {
    let board_edge = if props.top { 80.0 } else { 720.0 };
    let point_tip = if props.top { 350.0 } else { 450.0 };

    let triangle_points = format!(
        "{},{} {},{} {},{}",
        props.x - 35.0,
        board_edge,
        props.x + 35.0,
        board_edge,
        props.x,
        point_tip
    );

    let point_fill = if props.point.index % 2 == 0 {
        "#b7653b"
    } else {
        "#e5b66f"
    };

    let point_number_y = if props.top { 101.0 } else { 704.0 };

    let checker_count = usize::from(props.point.count);
    let checker_spacing = if checker_count <= 5 {
        45.0
    } else {
        225.0 / checker_count.saturating_sub(1) as f32
    };

    let point_index =
        u8::try_from(props.point.index).expect("projected point index must fit in u8");

    let source = MoveSource::Point(point_index);
    let destination = MoveTarget::Point(point_index);

    let on_source = {
        let callback = props.on_source.clone();

        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            callback.emit(source);
        })
    };

    let on_destination = {
        let callback = props.on_destination.clone();

        Callback::from(move |_| {
            callback.emit(destination);
        })
    };

    let destination_click = if props.destination_legal {
        on_destination
    } else {
        Callback::noop()
    };

    let checker_click = if props.source_selectable {
        on_source
    } else {
        Callback::noop()
    };

    let checkers = props.point.owner.map_or_else(
        || html! {},
        |player| {
            let player_name = match player {
                Player::White => "White",
                Player::Black => "Black",
            };

            html! {
                <>
                    {
                        for (0..props.point.count).map(|stack_index| {
                            let offset = f32::from(stack_index) * checker_spacing;

                            let y = if props.top {
                                125.0 + offset
                            } else {
                                675.0 - offset
                            };

                            html! {
                                <Checker
                                    player={player}
                                    x={props.x}
                                    y={y}
                                    label={format!(
                                        "{player_name} checker {} of {} on point {}",
                                        stack_index + 1,
                                        props.point.count,
                                        props.point.index + 1
                                    )}
                                    selectable={props.source_selectable}
                                    selected={props.source_selected}
                                    onclick={checker_click.clone()}
                                />
                            }
                        })
                    }
                </>
            }
        },
    );

    html! {
        <g
            class={classes!(
                "board-point",
                props.source_selectable.then_some("source-selectable"),
                props.source_selected.then_some("source-selected"),
                props.destination_legal.then_some("destination-legal"),
            )}
        >
            <polygon
                class="point-triangle"
                points={triangle_points.clone()}
                fill={point_fill}
                stroke="#3b2114"
                stroke-width="1.5"
                onclick={destination_click}
            />

            <polygon
                class="destination-highlight"
                points={triangle_points}
                pointer-events="none"
            />

            <text
                x={props.x.to_string()}
                y={point_number_y.to_string()}
                text-anchor="middle"
                font-size="13"
                font-weight="700"
                fill="#2a170e"
                pointer-events="none"
            >
                { props.point.index + 1 }
            </text>

            { checkers }
        </g>
    }
}
