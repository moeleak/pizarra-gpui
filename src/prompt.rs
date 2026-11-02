//! A themed replacement for GPUI's fallback prompt, used for the
//! "save changes?" questions and error messages. Enter picks the first button,
//! Escape the last one (always the "cancel"-like answer here).

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, KeyDownEvent, PromptButton, PromptHandle,
    PromptLevel, PromptResponse, RenderablePromptHandle, SharedString, Window, div, prelude::*, px,
};

use crate::theme;

pub fn build(
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let view = cx.new(|cx| PromptView {
        level,
        message: message.to_string().into(),
        detail: detail.map(|d| d.to_string().into()),
        actions: actions.iter().map(|a| a.label().clone()).collect(),
        focus: cx.focus_handle(),
    });
    handle.with_view(view, window, cx)
}

struct PromptView {
    level: PromptLevel,
    message: SharedString,
    detail: Option<SharedString>,
    actions: Vec<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<PromptResponse> for PromptView {}

impl Focusable for PromptView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PromptView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let last = self.actions.len().saturating_sub(1);
        let title_color = match self.level {
            PromptLevel::Critical => theme::danger(),
            _ => theme::text(),
        };

        let buttons = self.actions.iter().enumerate().map(|(ix, label)| {
            let primary = ix == 0;
            div()
                .id(ix)
                .px_4()
                .py_1p5()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .when(primary, |b| b.bg(theme::accent()).text_color(theme::accent_text()).font_weight(FontWeight::SEMIBOLD))
                .when(!primary, |b| b.bg(theme::hover()).hover(|s| s.bg(theme::pressed())))
                .child(label.clone())
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(ix))))
        });

        let card = div()
            .id("prompt-card")
            .occlude()
            .w(px(380.))
            .p_5()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .bg(theme::popover_bg())
            .border_1()
            .border_color(theme::border())
            .shadow_lg()
            .text_color(theme::text())
            .child(div().text_color(title_color).font_weight(FontWeight::SEMIBOLD).child(self.message.clone()))
            .children(self.detail.clone().map(|d| div().text_sm().text_color(theme::muted()).child(d)))
            // GNOME order: the primary answer sits at the right edge.
            .child(div().mt_3().flex().justify_end().gap_2().children(buttons.rev()));

        div()
            .id("prompt")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "enter" => cx.emit(PromptResponse(0)),
                "escape" => cx.emit(PromptResponse(last)),
                _ => {}
            }))
            .occlude()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .child(card)
    }
}
