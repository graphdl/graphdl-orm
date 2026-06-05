//! POST /input command parser for the see-and-drive surface (drive-half,
//! task kernel-see-drive-surface). Parses a tiny line-oriented command
//! language into pointer `InputAction`s that the lib.rs handler feeds to
//! the kernel's pointer ring / cursor position via
//! `pointer::set_position` + `pointer::push_pointer_event`.
//!
//! Pure + host-tested -- the parser holds the edge cases (bad args,
//! unknown verbs), so it gets strict RED-first TDD; the handler that
//! applies the actions is thin glue.
//!
//! Grammar (one command per line; blank lines + unknown verbs skipped):
//!   move <x> <y>                      place the cursor at screen px (x, y)
//!   click <left|right|middle>         press + release in one command
//!   button <left|right|middle> <down|up>
//!   scroll <delta>

use alloc::vec::Vec;

/// Linux input-event button codes (the consumer-side drain maps these to
/// `slint::PointerEventButton`). Kept here so the parser is self-contained.
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// A single decoded drive action. `Move` lands as a direct cursor-position
/// set (screen pixels); the rest land as `PointerEvent`s on the ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    Move { x: i32, y: i32 },
    Button { button: u32, pressed: bool },
    Scroll { delta: i32 },
}

fn button_code(name: &str) -> Option<u32> {
    match name {
        "left" => Some(BTN_LEFT),
        "right" => Some(BTN_RIGHT),
        "middle" => Some(BTN_MIDDLE),
        _ => None,
    }
}

/// Parse the POST /input body into a flat action list. Lenient: blank
/// lines, unknown verbs, and malformed args are skipped rather than
/// erroring, so a partially-valid batch still drives what it can.
pub fn parse_input(body: &str) -> Vec<InputAction> {
    let mut out = Vec::new();
    for line in body.lines() {
        let mut it = line.split_whitespace();
        let verb = match it.next() {
            Some(v) => v,
            None => continue, // blank line
        };
        match verb {
            "move" => {
                let x = it.next().and_then(|s| s.parse::<i32>().ok());
                let y = it.next().and_then(|s| s.parse::<i32>().ok());
                if let (Some(x), Some(y)) = (x, y) {
                    out.push(InputAction::Move { x, y });
                }
            }
            "click" => {
                if let Some(code) = it.next().and_then(button_code) {
                    out.push(InputAction::Button { button: code, pressed: true });
                    out.push(InputAction::Button { button: code, pressed: false });
                }
            }
            "button" => {
                let code = it.next().and_then(button_code);
                match (code, it.next()) {
                    (Some(code), Some("down")) => {
                        out.push(InputAction::Button { button: code, pressed: true })
                    }
                    (Some(code), Some("up")) => {
                        out.push(InputAction::Button { button: code, pressed: false })
                    }
                    _ => {}
                }
            }
            "scroll" => {
                if let Some(d) = it.next().and_then(|s| s.parse::<i32>().ok()) {
                    out.push(InputAction::Scroll { delta: d });
                }
            }
            _ => {} // unknown verb -> skip
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_move_then_click_into_actions() {
        let actions = parse_input("move 640 400\nclick left\n");
        assert_eq!(
            actions,
            alloc::vec![
                InputAction::Move { x: 640, y: 400 },
                InputAction::Button { button: BTN_LEFT, pressed: true },
                InputAction::Button { button: BTN_LEFT, pressed: false },
            ]
        );
    }

    #[test]
    fn button_down_up_and_scroll_parse() {
        let actions = parse_input("button right down\nscroll -3");
        assert_eq!(
            actions,
            alloc::vec![
                InputAction::Button { button: BTN_RIGHT, pressed: true },
                InputAction::Scroll { delta: -3 },
            ]
        );
    }

    #[test]
    fn skips_blank_and_unknown_and_malformed_lines() {
        let actions = parse_input("\nbogus 1 2\nmove 10\nmove 5 7\n");
        // "move 10" is malformed (one arg) -> skipped; only the full one lands.
        assert_eq!(actions, alloc::vec![InputAction::Move { x: 5, y: 7 }]);
    }
}
