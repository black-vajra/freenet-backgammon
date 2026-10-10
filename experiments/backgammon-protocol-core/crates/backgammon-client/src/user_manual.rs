//! Static, locally bundled help. No game, transport, or identity state is changed.
use yew::prelude::*;

#[function_component(UserManual)]
pub fn user_manual() -> Html {
    let open = use_state(|| false);
    let opener = use_node_ref();
    let heading = use_node_ref();

    let close = {
        let open = open.clone();
        let opener = opener.clone();
        Callback::from(move |()| {
            open.set(false);
            if let Some(button) = opener.cast::<web_sys::HtmlElement>() {
                let _ = button.focus();
            }
        })
    };
    let toggle = {
        let open = open.clone();
        let close = close.clone();
        Callback::from(move |_: MouseEvent| {
            if *open { close.emit(()); } else { open.set(true); }
        })
    };
    let hide = {
        let close = close.clone();
        Callback::from(move |_: MouseEvent| close.emit(()))
    };
    let escape = {
        let close = close.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                event.prevent_default();
                event.stop_propagation();
                close.emit(());
            }
        })
    };
    {
        let heading = heading.clone();
        use_effect_with(*open, move |is_open| {
            if *is_open {
                if let Some(title) = heading.cast::<web_sys::HtmlElement>() {
                    let _ = title.focus();
                }
            }
            || ()
        });
    }

    // Only this compile-time, project-owned HTML is trusted. Never substitute
    // player names, messages, or network content into from_html_unchecked.
    let contents = Html::from_html_unchecked(AttrValue::from(include_str!("user-manual-content.html")));
    html! {
        <div class="manual-widget">
            <button type="button" ref={opener}
                class="manual-toggle" aria-controls="user-manual-panel"
                aria-expanded={(*open).to_string()} onclick={toggle}>
                {if *open { "Hide user manual" } else { "User manual" }}
            </button>
            <section id="user-manual-panel" class="panel user-manual"
                aria-labelledby="user-manual-title" hidden={!*open} onkeydown={escape}>
                <div class="manual-heading-row">
                    <h2 id="user-manual-title" tabindex="-1" ref={heading}>{"Freenet Backgammon — User manual"}</h2>
                    <button type="button" onclick={hide.clone()}>{"Hide manual"}</button>
                </div>
                <div class="manual-body" tabindex="0" role="region" aria-label="User manual instructions">
                    {contents}
                </div>
                <div class="manual-footer">
                    <a href="user-manual.html" target="_blank" rel="noopener noreferrer">{"Open printable manual"}</a>
                    <button type="button" onclick={hide}>{"Hide manual"}</button>
                </div>
            </section>
        </div>
    }
}
