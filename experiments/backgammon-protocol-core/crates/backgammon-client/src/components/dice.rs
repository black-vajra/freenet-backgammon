use backgammon_core::Dice;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct DiceDisplayProps {
    pub dice: Option<Dice>,
    pub remaining: Vec<u8>,
    pub preview: bool,
}

#[function_component(DiceDisplay)]
pub fn dice_display(props: &DiceDisplayProps) -> Html {
    let values = props.dice.map(|dice| (dice.first, dice.second));

    html! {
        <section class="panel dice-panel" aria-labelledby="dice-heading">
            <h2 id="dice-heading">{ "Dice" }</h2>

            <div class="dice-row">
                <div class="die" aria-label={values.map_or(
                    "First die not rolled".to_owned(),
                    |(first, _)| format!("First die: {first}")
                )}>
                    { values.map_or("–".to_owned(), |(first, _)| first.to_string()) }
                </div>

                <div class="die" aria-label={values.map_or(
                    "Second die not rolled".to_owned(),
                    |(_, second)| format!("Second die: {second}")
                )}>
                    { values.map_or("–".to_owned(), |(_, second)| second.to_string()) }
                </div>
            </div>

            <p class="panel-note">
                {
                    if props.preview {
                        "Local move preview — awaiting authoritative confirmation"
                    } else if values.is_some() {
                        "Current verified roll"
                    } else {
                        "Awaiting roll"
                    }
                }
            </p>
            {
                props.dice.map_or_else(|| html! {}, |_| html! {
                    <p class="panel-note">
                        { format!("Dice remaining: {}", if props.remaining.is_empty() {
                            "none".to_owned()
                        } else {
                            props.remaining.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
                        }) }
                    </p>
                })
            }
        </section>
    }
}
