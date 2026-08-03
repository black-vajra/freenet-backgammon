use yew::prelude::*;

use crate::components::point::Point;
use crate::projection::BoardView;

#[derive(Properties, PartialEq)]
pub struct BoardProps {
    pub board: BoardView,
}

fn point_position(index: usize) -> (f32, bool) {
    const LEFT: [f32; 6] = [110.0, 180.0, 250.0, 320.0, 390.0, 460.0];
    const RIGHT: [f32; 6] = [740.0, 810.0, 880.0, 950.0, 1020.0, 1090.0];

    match index {
        // Points 1-6: bottom-right, numbered from the outer edge toward the bar.
        0..=5 => (RIGHT[5 - index], false),

        // Points 7-12: bottom-left, numbered from the bar toward the outer edge.
        6..=11 => (LEFT[11 - index], false),

        // Points 13-18: top-left, numbered from the outer edge toward the bar.
        12..=17 => (LEFT[index - 12], true),

        // Points 19-24: top-right, numbered from the bar toward the outer edge.
        18..=23 => (RIGHT[index - 18], true),

        _ => unreachable!("backgammon point index must be between 0 and 23"),
    }
}

#[function_component(Board)]
pub fn board(props: &BoardProps) -> Html {
    html! {
        <svg
            class="backgammon-board"
            viewBox="0 0 1200 800"
            width="100%"
            role="img"
            aria-label="Backgammon board showing the standard starting position"
        >
            <title>{ "Backgammon board" }</title>

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

            <rect
                x="555"
                y="55"
                width="90"
                height="690"
                fill="#321b0e"
                stroke="#211008"
                stroke-width="4"
            />

            {
                for props.board.points.iter().map(|point| {
                    let (x, top) = point_position(point.index);

                    html! {
                        <Point
                            point={*point}
                            x={x}
                            top={top}
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
