//! The menu-bar icon and menu. The icon is drawn in code, so no runtime asset is loaded:
//! a three-picket fence, as a template image that follows the menu bar's appearance,
//! filled amber or red when items need attention. Colour is never the only signal; the
//! first menu line says the same in words.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
};

pub const TRAY_ID: &str = "picket";
/// Drawn at twice the menu bar's 18-point height for Retina displays.
const SIZE: usize = 36;
const AMBER: [u8; 3] = [0xE8, 0x9A, 0x0C];
const RED: [u8; 3] = [0xE5, 0x48, 0x4D];
/// Menu titles longer than this are shortened.
const MAX_TITLE: usize = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconState {
    /// Nothing open in the sources Picket can see.
    Quiet,
    Review,
    Urgent,
    /// Alerts paused, or Google not connected with nothing open.
    Muted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct View {
    pub state: IconState,
    /// Status lines shown as disabled items at the top of the menu.
    pub lines: Vec<String>,
    /// Up to three open incidents, newest first: id and title.
    pub open: Vec<(String, String)>,
    pub paused: bool,
}

/// The icon and menu shown before the first status is known.
pub fn initial() -> View {
    View {
        state: IconState::Quiet,
        lines: vec!["Starting…".into()],
        open: Vec::new(),
        paused: false,
    }
}

pub fn apply(app: &tauri::AppHandle, view: &View) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    tray.set_icon(Some(icon(view.state)))?;
    tray.set_icon_as_template(matches!(view.state, IconState::Quiet | IconState::Muted))?;
    tray.set_tooltip(view.lines.first().map(|line| format!("Picket - {line}")))?;
    tray.set_menu(Some(menu(app, view)?))?;
    Ok(())
}

fn menu(app: &tauri::AppHandle, view: &View) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for (index, line) in view.lines.iter().enumerate() {
        menu.append(&MenuItem::with_id(
            app,
            format!("status-{index}"),
            line,
            false,
            None::<&str>,
        )?)?;
    }
    if !view.open.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        for (id, title) in &view.open {
            menu.append(&MenuItem::with_id(
                app,
                id,
                shorten(title),
                true,
                None::<&str>,
            )?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "open",
        "Open Picket",
        true,
        None::<&str>,
    )?)?;
    let (pause_id, pause_label) = if view.paused {
        ("resume", "Resume alerts")
    } else {
        ("pause", "Pause alerts for 1 hour")
    };
    menu.append(&MenuItem::with_id(
        app,
        pause_id,
        pause_label,
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit Picket",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn shorten(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE {
        return title.to_owned();
    }
    let cut: String = title.chars().take(MAX_TITLE - 1).collect();
    format!("{}…", cut.trim_end())
}

pub fn icon(state: IconState) -> Image<'static> {
    let (rgb, alpha) = match state {
        IconState::Quiet => ([0, 0, 0], 255),
        // A template image keeps its alpha, so muted reads as a dimmed fence.
        IconState::Muted => ([0, 0, 0], 110),
        IconState::Review => (AMBER, 255),
        IconState::Urgent => (RED, 255),
    };
    let mut rgba = vec![0; SIZE * SIZE * 4];
    for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        if fence(index % SIZE, index / SIZE) {
            *pixel = [rgb[0], rgb[1], rgb[2], alpha];
        }
    }
    Image::new_owned(rgba, SIZE as u32, SIZE as u32)
}

/// Three pickets with pointed tops on two rails, centred in the image.
/// `from_center` is in half pixels, so the tip row is two pixels wide.
fn fence(x: usize, y: usize) -> bool {
    const PICKETS: [usize; 3] = [5, 15, 25];
    const WIDTH: usize = 6;
    let picket = PICKETS.iter().any(|&left| {
        if !(left..left + WIDTH).contains(&x) {
            return false;
        }
        let from_center = (2 * x + 1).abs_diff(2 * left + WIDTH);
        match y {
            // The point widens by two pixels per row.
            4..=6 => from_center < 2 * (y - 4) + 2,
            7..=32 => true,
            _ => false,
        }
    });
    let rail = (PICKETS[0]..PICKETS[2] + WIDTH).contains(&x)
        && ((13..16).contains(&y) || (24..27).contains(&y));
    picket || rail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fence_is_symmetric_and_pointed() {
        let rows: Vec<String> = (0..SIZE)
            .map(|y| {
                (0..SIZE)
                    .map(|x| if fence(x, y) { '#' } else { '.' })
                    .collect()
            })
            .collect();
        for row in &rows {
            assert_eq!(*row, row.chars().rev().collect::<String>(), "{row}");
        }
        assert_eq!(&rows[4][5..11], "..##..");
        assert_eq!(&rows[5][5..11], ".####.");
        assert_eq!(&rows[6][5..11], "######");
        assert_eq!(rows[3], ".".repeat(SIZE));
        assert_eq!(rows[33], ".".repeat(SIZE));
    }

    #[test]
    fn long_titles_are_shortened() {
        let title = "x".repeat(80);
        assert_eq!(shorten(&title).chars().count(), MAX_TITLE);
        assert!(shorten(&title).ends_with('…'));
        assert_eq!(shorten("Short"), "Short");
    }
}
