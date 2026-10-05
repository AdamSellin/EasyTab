//! macOS : le terminal est la fenêtre au premier plan (liste des fenêtres de
//! Core Graphics, sans autorisation). Son curseur vient de l'accessibilité
//! (`AXBoundsForRange` du texte sélectionné de l'élément qui a le focus)
//! quand EasyTab y a accès ; sinon il est déduit du cadre de la fenêtre et de
//! la taille de la grille (`estimate`). Les coordonnées sont en points,
//! depuis le coin haut gauche de l'écran principal, comme celles de
//! l'accessibilité : un pixel CSS de la page vaut un point.

use std::hash::{Hash, Hasher};
use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWindow};
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRange, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerPID,
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event_loop::{EventLoop, EventLoopBuilder};
use tao::platform::macos::{
    ActivationPolicy, EventLoopExtMacOS, WindowBuilderExtMacOS, WindowExtMacOS,
};
use tao::window::{Window, WindowBuilder};
use wry::{WebView, WebViewBuilder};

use crate::estimate::{caret_from_frame, Hint};
use crate::placement::{Caret, Rect};

/// Hauteur de la barre de titre d'une fenêtre, en points.
const TITLE_BAR: i32 = 28;
/// Plus grande case de texte crédible, en points.
const MAX_CELL: f64 = 120.0;

pub fn event_loop<T: 'static>() -> anyhow::Result<EventLoop<T>> {
    let mut event_loop = EventLoopBuilder::<T>::with_user_event().build();
    // Ni icône dans le Dock, ni menu : le programme ne devient jamais actif.
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    Ok(event_loop)
}

pub fn configure(builder: WindowBuilder) -> WindowBuilder {
    builder
        .with_focusable(false)
        .with_visible_on_all_workspaces(true)
        // La page dessine sa propre ombre autour du cadre.
        .with_has_shadow(false)
}

pub fn webview<'a>(builder: WebViewBuilder<'a>, window: &'a Window) -> wry::Result<WebView> {
    builder.build(window)
}

pub struct Surface;

impl Surface {
    pub fn new(window: &Window) -> Self {
        let _ = window.set_ignore_cursor_events(true);
        Self
    }

    pub fn show(&self, window: &Window, x: i32, y: i32, width: i32, height: i32) {
        window.set_inner_size(LogicalSize::new(width as f64, height as f64));
        window.set_outer_position(LogicalPosition::new(x as f64, y as f64));
        // `set_visible` passerait par `makeKeyAndOrderFront` : on montre la
        // fenêtre sans la rendre active ni activer le programme.
        // SAFETY: `ns_window` est notre fenêtre, vivante tant que `window`
        // l'est ; appel depuis le fil principal (boucle d'événements).
        unsafe {
            let ns_window = &*(window.ns_window() as *const NSWindow);
            ns_window.orderFrontRegardless();
        }
    }

    pub fn hide(&self, window: &Window) {
        window.set_visible(false);
    }

    /// Zone utilisable (sans la barre des menus ni le Dock) de l'écran qui
    /// contient le curseur, en points. L'échelle vaut 1 : la page compte en
    /// points elle aussi.
    pub fn work_area(&self, _window: &Window, caret: Rect) -> (Rect, f64) {
        let Some(mtm) = MainThreadMarker::new() else {
            return (Rect::EVERYWHERE, 1.0);
        };
        let screens = NSScreen::screens(mtm);
        // AppKit compte depuis le bas de l'écran principal.
        let Some(primary) = screens.firstObject() else {
            return (Rect::EVERYWHERE, 1.0);
        };
        let height = primary.frame().size.height;
        let flip = |rect: CGRect| Rect {
            left: rect.origin.x.round() as i32,
            top: (height - rect.origin.y - rect.size.height).round() as i32,
            right: (rect.origin.x + rect.size.width).round() as i32,
            bottom: (height - rect.origin.y).round() as i32,
        };
        for screen in screens.iter() {
            if flip(screen.frame()).contains(caret.left, caret.top) {
                return (flip(screen.visibleFrame()), 1.0);
            }
        }
        (Rect::EVERYWHERE, 1.0)
    }
}

pub struct Locator;

impl Locator {
    pub fn new() -> Self {
        Self
    }

    pub fn locate(&self, hint: Hint) -> Option<Caret> {
        let front = front_window()?;
        // SAFETY: appels d'accessibilité en lecture ; les valeurs rendues
        // sont vérifiées avant usage.
        let (focused, caret, frame) = unsafe {
            if AXIsProcessTrusted() {
                let system = AXUIElement::new_system_wide();
                // Un terminal bloqué ne doit pas bloquer la recherche.
                let _ = system.set_messaging_timeout(0.25);
                let focused = attribute::<AXUIElement>(&system, "AXFocusedUIElement");
                let caret = focused
                    .as_ref()
                    .and_then(|element| text_caret(element))
                    .filter(|(rect, _)| front.bounds.contains(rect.left, rect.top));
                let frame = attribute::<AXUIElement>(&system, "AXFocusedApplication")
                    .and_then(|app| attribute::<AXUIElement>(&app, "AXFocusedWindow"))
                    .and_then(|window| attribute::<AXValue>(&window, "AXFrame"))
                    .and_then(|frame| rect_value(&frame));
                (focused, caret, frame)
            } else {
                (None, None, None)
            }
        };
        let (source, (rect, cell_width)) = match caret {
            Some(caret) => ("accessibilité", caret),
            None => (
                "fenêtre",
                caret_from_frame(frame.unwrap_or(front.bounds), TITLE_BAR, hint)?,
            ),
        };
        let caret = Caret {
            rect,
            cell_width,
            window: front.number,
            element: focused.map_or(0, |element| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                element.hash(&mut hasher);
                hasher.finish()
            }),
        };
        super::log(source, &caret);
        Some(caret)
    }
}

/// Fenêtre au premier plan.
pub fn foreground() -> isize {
    front_window().map_or(0, |front| front.number)
}

/// Fenêtre normale (niveau 0) la plus en avant, hors les nôtres.
struct Front {
    number: isize,
    bounds: Rect,
}

fn front_window() -> Option<Front> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID)?;
    // SAFETY: la liste est un tableau de dictionnaires à clés texte
    // (documentation de `CGWindowListCopyWindowInfo`) ; les clés sont des
    // constantes de Core Graphics.
    let (list, layer_key, pid_key, number_key, bounds_key) = unsafe {
        (
            CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(list),
            kCGWindowLayer,
            kCGWindowOwnerPID,
            kCGWindowNumber,
            kCGWindowBounds,
        )
    };
    let own = i64::from(std::process::id());
    for info in list.iter() {
        let number = |key: &CFString| info.get(key)?.downcast::<CFNumber>().ok()?.as_i64();
        if number(layer_key) != Some(0) || number(pid_key) == Some(own) {
            continue;
        }
        let Some(bounds) = info
            .get(bounds_key)
            .and_then(|bounds| bounds.downcast::<CFDictionary>().ok())
        else {
            continue;
        };
        let mut rect = CGRect::default();
        // SAFETY: `rect` vit pendant l'appel.
        if !unsafe { CGRectMakeWithDictionaryRepresentation(Some(&bounds), &mut rect) } {
            continue;
        }
        return Some(Front {
            number: number(number_key)? as isize,
            bounds: to_rect(rect),
        });
    }
    None
}

/// Attribut `name` de l'élément, s'il est du type `T`.
unsafe fn attribute<T: objc2_core_foundation::ConcreteType>(
    element: &AXUIElement,
    name: &'static str,
) -> Option<CFRetained<T>> {
    let name = CFString::from_static_str(name);
    let mut value: *const CFType = std::ptr::null();
    if element.copy_attribute_value(&name, NonNull::from(&mut value)) != AXError::Success {
        return None;
    }
    let value = CFRetained::from_raw(NonNull::new(value.cast_mut())?);
    value.downcast::<T>().ok()
}

/// Case du caractère sous le curseur de texte de l'élément. En fin de ligne,
/// il n'y a souvent pas de caractère : on prend celui d'avant et on se place
/// juste après.
unsafe fn text_caret(element: &AXUIElement) -> Option<(Rect, f64)> {
    let selection = attribute::<AXValue>(element, "AXSelectedTextRange")?;
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    if !selection.value(AXValueType::CFRange, NonNull::from(&mut range).cast()) {
        return None;
    }
    if let Some(rect) = character_bounds(element, range.location) {
        return Some((rect, (rect.right - rect.left) as f64));
    }
    let previous = character_bounds(element, range.location.checked_sub(1)?)?;
    let width = previous.right - previous.left;
    Some((
        Rect {
            left: previous.right,
            right: previous.right + width,
            ..previous
        },
        width as f64,
    ))
}

/// Rectangle à l'écran du caractère en position `location`.
unsafe fn character_bounds(element: &AXUIElement, location: isize) -> Option<Rect> {
    if location < 0 {
        return None;
    }
    let mut range = CFRange {
        location,
        length: 1,
    };
    let parameter = AXValue::new(AXValueType::CFRange, NonNull::from(&mut range).cast())?;
    let name = CFString::from_static_str("AXBoundsForRange");
    let mut value: *const CFType = std::ptr::null();
    let error =
        element.copy_parameterized_attribute_value(&name, &parameter, NonNull::from(&mut value));
    if error != AXError::Success {
        return None;
    }
    let value = CFRetained::from_raw(NonNull::new(value.cast_mut())?)
        .downcast::<AXValue>()
        .ok()?;
    let rect = rect_value(&value)?;
    let (width, height) = (
        (rect.right - rect.left) as f64,
        (rect.bottom - rect.top) as f64,
    );
    let credible = |size: f64| size > 0.0 && size <= MAX_CELL;
    (credible(width) && credible(height)).then_some(rect)
}

/// Rectangle contenu dans une valeur d'accessibilité.
unsafe fn rect_value(value: &AXValue) -> Option<Rect> {
    let mut rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
    value
        .value(AXValueType::CGRect, NonNull::from(&mut rect).cast())
        .then(|| to_rect(rect))
}

fn to_rect(rect: CGRect) -> Rect {
    Rect {
        left: rect.origin.x.round() as i32,
        top: rect.origin.y.round() as i32,
        right: (rect.origin.x + rect.size.width).round() as i32,
        bottom: (rect.origin.y + rect.size.height).round() as i32,
    }
}
