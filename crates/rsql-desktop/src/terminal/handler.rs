use alacritty_terminal::Term;
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::term::color::COUNT;
use alacritty_terminal::vte::ansi::{self, Handler};

use super::{Rgb, events::Events};

/// Keep the pinned library's complete Handler surface, resolving color replies
/// before a later command in the same dispatch or synchronized batch changes it.
pub(super) struct QueryTimeHandler<'a> {
    pub mouse_generation: &'a mut u64,
    pub term: &'a mut Term<Events>,
    pub events: &'a Events,
    pub palette: &'a [Rgb; COUNT],
    pub size: WindowSize,
}

macro_rules! forward {
    ($($method:ident($($argument:ident: $ty:ty),*);)*) => {
        $(fn $method(&mut self, $($argument: $ty),*) {
            self.term.$method($($argument),*);
        })*
    };
}

impl Handler for QueryTimeHandler<'_> {
    forward! {
        set_title(title: Option<String>);
        set_cursor_style(style: Option<ansi::CursorStyle>);
        set_cursor_shape(shape: ansi::CursorShape);
        input(character: char);
        goto(line: i32, column: usize);
        goto_line(line: i32);
        goto_col(column: usize);
        insert_blank(count: usize);
        move_up(rows: usize);
        move_down(rows: usize);
        identify_terminal(intermediate: Option<char>);
        device_status(status: usize);
        move_forward(columns: usize);
        move_backward(columns: usize);
        move_down_and_cr(rows: usize);
        move_up_and_cr(rows: usize);
        put_tab(count: u16);
        backspace();
        carriage_return();
        linefeed();
        bell();
        substitute();
        newline();
        set_horizontal_tabstop();
        scroll_up(rows: usize);
        scroll_down(rows: usize);
        insert_blank_lines(count: usize);
        delete_lines(count: usize);
        erase_chars(count: usize);
        delete_chars(count: usize);
        move_backward_tabs(count: u16);
        move_forward_tabs(count: u16);
        save_cursor_position();
        restore_cursor_position();
        clear_line(mode: ansi::LineClearMode);
        clear_screen(mode: ansi::ClearMode);
        clear_tabs(mode: ansi::TabulationClearMode);
        set_tabs(interval: u16);
        reset_state();
        reverse_index();
        terminal_attribute(attribute: ansi::Attr);
        set_mode(mode: ansi::Mode);
        unset_mode(mode: ansi::Mode);
        report_mode(mode: ansi::Mode);
        unset_private_mode(mode: ansi::PrivateMode);
        report_private_mode(mode: ansi::PrivateMode);
        set_scrolling_region(top: usize, bottom: Option<usize>);
        set_keypad_application_mode();
        unset_keypad_application_mode();
        set_active_charset(index: ansi::CharsetIndex);
        configure_charset(index: ansi::CharsetIndex, charset: ansi::StandardCharset);
        set_color(index: usize, color: Rgb);
        reset_color(index: usize);
        clipboard_store(clipboard: u8, base64: &[u8]);
        clipboard_load(clipboard: u8, terminator: &str);
        decaln();
        push_title();
        pop_title();
        text_area_size_pixels();
        text_area_size_chars();
        set_hyperlink(hyperlink: Option<ansi::Hyperlink>);
        set_mouse_cursor_icon(icon: ansi::cursor_icon::CursorIcon);
        report_keyboard_mode();
        push_keyboard_mode(mode: ansi::KeyboardModes);
        pop_keyboard_modes(count: u16);
        set_keyboard_mode(mode: ansi::KeyboardModes, behavior: ansi::KeyboardModesApplyBehavior);
        set_modify_other_keys(mode: ansi::ModifyOtherKeys);
        report_modify_other_keys();
        set_scp(path: ansi::ScpCharPath, mode: ansi::ScpUpdateMode);
    }

    fn set_private_mode(&mut self, mode: ansi::PrivateMode) {
        let active = |mode: super::TermMode| {
            mode.contains(super::TermMode::SGR_MOUSE)
                && mode.intersects(super::TermMode::MOUSE_MODE)
        };
        let was_active = active(*self.term.mode());
        self.term.set_private_mode(mode);
        // A program can disable and re-enable reporting within one published frame.
        if !was_active && active(*self.term.mode()) {
            *self.mouse_generation = self.mouse_generation.wrapping_add(1);
        }
    }

    fn dynamic_color_sequence(&mut self, prefix: String, index: usize, terminator: &str) {
        self.term.dynamic_color_sequence(prefix, index, terminator);
        self.events
            .lock()
            .resolve(self.term.colors(), self.palette, self.size);
    }
}
