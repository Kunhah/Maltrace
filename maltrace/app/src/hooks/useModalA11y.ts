import { useEffect, type RefObject } from "react";

/// Focus + keyboard hygiene for modal dialogs.
///
/// On mount: stashes the previously-focused element, then focuses the
/// first focusable inside `containerRef`. On unmount: restores focus
/// to the original element (so the topbar trigger is re-highlighted
/// after Esc-closing a modal). While mounted: Tab loops within the
/// modal (Tab past last → first; Shift+Tab past first → last) and
/// `Escape` calls `onClose`.
///
/// Behaviour mirrors WAI-ARIA Authoring Practices for dialog
/// patterns. We don't ARIA-modal the document because maltrace's
/// "modals" coexist with the canvas (the inspector still updates
/// behind them) — they're more like persistent floating cards than
/// truly modal interruptions. Focus-trapping is still the right
/// default though.
export function useModalA11y(
  containerRef: RefObject<HTMLElement | null>,
  onClose: () => void,
) {
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const previouslyFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;

    // Focus the first focusable on mount. If none exist, focus the
    // container itself so Esc still works.
    const focusables = collectFocusables(container);
    (focusables[0] ?? container).focus({ preventScroll: true });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== "Tab") return;
      const live = collectFocusables(container);
      if (live.length === 0) return;
      const first = live[0];
      const last = live[live.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    container.addEventListener("keydown", onKey);
    return () => {
      container.removeEventListener("keydown", onKey);
      // Restore focus only if the previously-focused element is
      // still in the DOM (it might have unmounted while the modal
      // was open).
      if (previouslyFocused && document.contains(previouslyFocused)) {
        previouslyFocused.focus({ preventScroll: true });
      }
    };
  }, [containerRef, onClose]);
}

/// Every visible, enabled, focusable element inside `root`. The list
/// is re-collected on every Tab press so dynamic content (e.g. an
/// input that appears after a checkbox) is handled correctly.
function collectFocusables(root: HTMLElement): HTMLElement[] {
  const selectors = [
    "a[href]",
    "button:not([disabled])",
    "input:not([disabled]):not([type=hidden])",
    "select:not([disabled])",
    "textarea:not([disabled])",
    '[tabindex]:not([tabindex="-1"])',
  ].join(",");
  return Array.from(root.querySelectorAll<HTMLElement>(selectors)).filter(
    (el) => el.offsetParent !== null || el === document.activeElement,
  );
}
