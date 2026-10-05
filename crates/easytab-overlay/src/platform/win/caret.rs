//! Position du curseur du terminal à l'écran, en pixels.
//!
//! Le terminal où l'on tape est la fenêtre au premier plan. On essaie, dans
//! l'ordre :
//! 1. l'élément qui a le focus, s'il a la taille d'une case : VS Code place
//!    sa zone de saisie, invisible, exactement sur le curseur. Sa position est
//!    calculée à la demande, donc à jour, alors que le curseur système de VS
//!    Code (Chromium) n'est déplacé qu'en retard et reste souvent à gauche ;
//! 2. UI Automation : le caractère sous le curseur de texte (Windows Terminal
//!    le fournit) ;
//! 3. le curseur système (`GetGUIThreadInfo`), que certaines applications
//!    tiennent à jour.

use std::hash::{Hash, Hasher};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Ole::{SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetUBound};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextPattern2, IUIAutomationTextRange, TextPatternRangeEndpoint_Start,
    TextUnit_Character, UIA_TextPattern2Id, UIA_TextPatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

use crate::estimate::Hint;
use crate::placement::{Caret, Rect};

/// Plus grande case de texte crédible, en pixels (polices agrandies, écrans
/// à forte densité compris).
const MAX_CELL: f64 = 120.0;

pub struct Locator {
    automation: Option<IUIAutomation>,
}

impl Locator {
    /// À créer dans le fil qui fera les appels (COM y est initialisé).
    pub fn new() -> Self {
        // SAFETY: initialisation COM du fil courant ; l'objet UI Automation
        // n'est utilisé que depuis ce fil.
        let automation = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        };
        Self { automation }
    }

    pub fn locate(&self, _hint: Hint) -> Option<Caret> {
        // SAFETY: appels Win32/COM en lecture ; les pointeurs passés vivent
        // pendant chaque appel.
        unsafe {
            let window = GetForegroundWindow();
            if window.is_invalid() {
                return None;
            }
            let focused = self
                .automation
                .as_ref()
                .and_then(|automation| automation.GetFocusedElement().ok());
            let (rect, cell_width, source) = if let Some((rect, cell)) =
                focused.as_ref().and_then(|element| cell_sized(element))
            {
                (rect, cell, "zone")
            } else if let Some((rect, cell)) =
                focused.as_ref().and_then(|element| text_caret(element))
            {
                (rect, cell, "uia")
            } else {
                let (rect, cell) = system_caret(window)?;
                (rect, cell, "système")
            };
            let caret = Caret {
                rect,
                cell_width,
                window: window.0 as isize,
                element: focused.as_ref().map_or(0, |element| element_id(element)),
            };
            crate::platform::log(source, &caret);
            Some(caret)
        }
    }
}

/// Fenêtre au premier plan.
pub fn foreground() -> isize {
    // SAFETY: lecture sans paramètre.
    unsafe { GetForegroundWindow().0 as isize }
}

unsafe fn system_caret(window: HWND) -> Option<(Rect, f64)> {
    let thread = GetWindowThreadProcessId(window, None);
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    GetGUIThreadInfo(thread, &mut info).ok()?;
    let caret = info.rcCaret;
    if info.hwndCaret.is_invalid() || caret.bottom <= caret.top {
        return None;
    }
    let mut top_left = POINT {
        x: caret.left,
        y: caret.top,
    };
    let mut bottom_right = POINT {
        x: caret.right,
        y: caret.bottom,
    };
    if !ClientToScreen(info.hwndCaret, &mut top_left).as_bool()
        || !ClientToScreen(info.hwndCaret, &mut bottom_right).as_bool()
    {
        return None;
    }
    let height = (bottom_right.y - top_left.y) as f64;
    // Le curseur système est un trait : une case fait environ la moitié de
    // sa hauteur en largeur.
    Some((
        Rect {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        },
        (height * 0.5).max(1.0),
    ))
}

/// Identifiant de l'élément qui a le focus : chaque onglet de Windows Terminal
/// et chaque terminal de VS Code a le sien.
unsafe fn element_id(element: &IUIAutomationElement) -> u64 {
    let Ok(array) = element.GetRuntimeId() else {
        return 0;
    };
    if array.is_null() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let upper = SafeArrayGetUBound(array, 1).unwrap_or(-1);
    for index in 0..=upper {
        let mut value = 0i32;
        if SafeArrayGetElement(array, &index, &mut value as *mut i32 as *mut _).is_ok() {
            value.hash(&mut hasher);
        }
    }
    let _ = SafeArrayDestroy(array);
    hasher.finish()
}

/// Curseur de texte UI Automation de l'élément.
unsafe fn text_caret(element: &IUIAutomationElement) -> Option<(Rect, f64)> {
    let range = element
        .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
        .ok()
        .and_then(|pattern| {
            let mut active = BOOL::default();
            pattern.GetCaretRange(&mut active).ok()
        })
        .or_else(|| {
            let pattern = element
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .ok()?;
            let selection = pattern.GetSelection().ok()?;
            (selection.Length().ok()? > 0)
                .then(|| selection.GetElement(0).ok())
                .flatten()
        });
    range.and_then(|range| cell_of(&range))
}

/// L'élément lui-même, s'il a la taille d'une case : la zone de saisie de VS
/// Code (xterm.js) est posée sur le curseur.
unsafe fn cell_sized(element: &IUIAutomationElement) -> Option<(Rect, f64)> {
    let bounds = element.CurrentBoundingRectangle().ok()?;
    let (width, height) = (
        (bounds.right - bounds.left) as f64,
        (bounds.bottom - bounds.top) as f64,
    );
    if width <= 0.0 || height <= 0.0 || height > MAX_CELL || width > MAX_CELL {
        return None;
    }
    let rect = to_rect(bounds);
    let cell = if width >= height * 0.3 {
        width
    } else {
        height * 0.5
    };
    Some((rect, cell))
}

/// Case du caractère sous le curseur. En fin de ligne, il n'y a souvent pas de
/// caractère : on prend celui d'avant et on se place juste après.
unsafe fn cell_of(caret: &IUIAutomationTextRange) -> Option<(Rect, f64)> {
    let range = caret.Clone().ok()?;
    if range.ExpandToEnclosingUnit(TextUnit_Character).is_ok() {
        if let Some(rect) = first_rect(&range) {
            let width = (rect.right - rect.left) as f64;
            return Some((rect, width));
        }
    }
    let range = caret.Clone().ok()?;
    range
        .MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1)
        .ok()?;
    let previous = first_rect(&range)?;
    let width = (previous.right - previous.left) as f64;
    Some((
        Rect {
            left: previous.right,
            top: previous.top,
            right: previous.right + width as i32,
            bottom: previous.bottom,
        },
        width,
    ))
}

/// Premier rectangle (x, y, largeur, hauteur) d'une plage de texte.
unsafe fn first_rect(range: &IUIAutomationTextRange) -> Option<Rect> {
    let array = range.GetBoundingRectangles().ok()?;
    if array.is_null() {
        return None;
    }
    let mut values = [0f64; 4];
    let count = SafeArrayGetUBound(array, 1)
        .map(|upper| upper + 1)
        .unwrap_or(0);
    let mut ok = count >= 4;
    if ok {
        for (index, value) in values.iter_mut().enumerate() {
            let index = index as i32;
            ok &= SafeArrayGetElement(array, &index, value as *mut f64 as *mut _).is_ok();
        }
    }
    let _ = SafeArrayDestroy(array);
    let [x, y, width, height] = values;
    let ok = ok && width > 0.0 && height > 0.0 && width <= MAX_CELL && height <= MAX_CELL;
    ok.then(|| Rect {
        left: x.round() as i32,
        top: y.round() as i32,
        right: (x + width).round() as i32,
        bottom: (y + height).round() as i32,
    })
}

fn to_rect(rect: RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}
