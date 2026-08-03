use backgammon_core::{MoveSource, Player};
use yew::prelude::*;

use crate::controller::LocalTurnRecord;

#[derive(Properties, PartialEq)]
pub struct MoveHistoryProps {
    pub history: Vec<LocalTurnRecord>,
}

#[function_component(MoveHistory)]
pub fn move_history(props: &MoveHistoryProps) -> Html {
    html! {
        <section class="panel history-panel" aria-labelledby="history-heading">
            <div class="panel-heading-row">
                <h2 id="history-heading">{ "Move history" }</h2>
                <span class="history-count">{ props.history.len() }</span>
            </div>

            <ol class="move-history">
                {
                    if props.history.is_empty() {
                        html! {
                            <li class="history-placeholder">
                                { "No moves yet. White begins." }
                            </li>
                        }
                    } else {
                        html! {
                            <>
                                {
                                    for props.history.iter().enumerate().map(|(index, turn)| {
                                        let player_name = match turn.player {
                                            Player::White => "White",
                                            Player::Black => "Black",
                                        };

                                        let moves = if turn.moves.is_empty() {
                                            "No legal move".to_owned()
                                        } else {
                                            turn.moves
                                                .iter()
                                                .map(|checker_move| {
                                                    let source = match checker_move.source {
                                                        MoveSource::Bar => "bar".to_owned(),
                                                        MoveSource::Point(point) => {
                                                            format!("point {}", point + 1)
                                                        }
                                                    };

                                                    format!("{source} +{}", checker_move.die)
                                                })
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        };

                                        html! {
                                            <li class="history-entry">
                                                <strong>
                                                    {
                                                        format!(
                                                            "{}. {player_name}",
                                                            index + 1
                                                        )
                                                    }
                                                </strong>

                                                <span>
                                                    {
                                                        format!(
                                                            "Rolled {}–{} · {moves}",
                                                            turn.dice.first,
                                                            turn.dice.second
                                                        )
                                                    }
                                                </span>
                                            </li>
                                        }
                                    })
                                }
                            </>
                        }
                    }
                }
            </ol>
        </section>
    }
}
