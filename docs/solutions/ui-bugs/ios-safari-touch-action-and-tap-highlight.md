---
title: iOS Safari touch-action:none breaks tap events; default tap highlight flashes white
category: ui-bugs
component: client/index.html
problem_type: platform_css_default
symptoms:
  - "iPad users can open Settings menu but can't exit it or select any item"
  - "White / washed-out overlay flashes over dark UI panels on weapon select and FIRE tap"
  - "Rapid double-taps on FIRE swallow the second shot"
severity: high
resolution: fixed
fixed_in: warships/7f784a6
affected_platforms: [ios_safari, ipad]
related_files:
  - client/index.html
tags: [ios, safari, ipad, css, touch-action, tap-highlight, kodiak-override]
---

# iOS Safari `touch-action: none` breaks tap events; default tap highlight flashes white

Two independent iPad-only bugs that shipped the same afternoon, both rooted
in platform-specific Safari defaults that the kodiak framework doesn't
override. Fixed together because both needed the same type of fix
(CSS override in our `index.html`) and we wanted one deploy instead of two.

## Bug 1: Settings menu trap

### Symptom

On iPad (Safari), tap the gear icon → Settings dialog opens → tap the
backdrop to dismiss → nothing. Tap a menu item inside the dialog →
nothing. User is trapped until they force-reload the page.

Direct quote from playtest: *"i open settings on ipad, i can't get out
of it and can't select any menu items."*

### Root cause

Kodiak's `app.rs` sets this globally:
```css
body { touch-action: none; }
```

This was intended to block browser-native gestures (pinch-to-zoom,
double-tap-zoom, pull-to-refresh) while the player has a finger on the
canvas. On iOS Safari specifically, `touch-action: none` has a side
effect: **it suppresses the synthesized `click` event** that would
normally fire on a `<div>` with an `onclick` handler when tapped.

Every kodiak `NexusDialog` wraps its content in a `Curtain` component —
a plain `<div>` with an `onclick` that dismisses the dialog. The
dismiss never fires. Menu items inside the dialog fail for the same
reason: they're `<div>`s, not `<button>`s, so `touch-action: none`
propagates to them and the click is never synthesized.

The bug did not reproduce in Chrome or Android WebView because those
browsers synthesize clicks from taps independently of `touch-action`.

### Fix

In `client/index.html`, override the body rule with `!important` (we
can't edit kodiak directly without rebuilding the framework dependency
chain):
```css
body {
  touch-action: manipulation !important;
  -webkit-tap-highlight-color: transparent !important;
  -webkit-touch-callout: none !important;
}
```

`touch-action: manipulation` still blocks double-tap-zoom (our original
reason for `none`), but preserves tap-to-click synthesis. It's the
correct value for "let the user tap UI elements, but don't let them
zoom or scroll via the canvas."

### Why this is a kodiak-level override, not an upstream fix

We don't want to change kodiak's default because the framework targets
multiple platforms. Our override lives in `client/index.html` as a
two-line CSS block, clearly commented, and we win on cascade order
with `!important`.

## Bug 2: White flash on every button tap

### Symptom

Tapping FIRE or any weapon button on iPad produces a brief
white/light-gray flash covering the button. Against our dark wargame
panels it reads as a "washed-out overlay" and feels broken.

Direct quote: *"the whole game UI gets a white / washed out layover
or something when selecting different weapons and when trying to shoot."*

### Root cause

iOS Safari's default for **every `<button>`, `<a>`, and clickable
element** is:
```css
-webkit-tap-highlight-color: rgba(0, 0, 0, 0.3);
```

On a light UI this is invisible. On dark panels (our case), it renders
as a translucent gray overlay for ~100 ms on every tap. Neither kodiak
nor warships had set this to `transparent` anywhere.

### Fix

Explicit override for every clickable element type, same `index.html`
block as bug 1:
```css
button, a, [role="button"],
input[type="button"], input[type="submit"] {
  -webkit-tap-highlight-color: transparent !important;
}
```

The `body`-level override handles plain `<div>`s; the element-specific
rule handles actual buttons and links. Both are needed because some
engines (kodiak included) set tap-highlight on individual elements
which wins over a `body` declaration.

## Bonus: removed a JS double-tap-zoom hack

While in the same file, we deleted this JavaScript:
```js
let lastTouch = 0;
window.addEventListener('touchend', e => {
  const now = Date.now();
  if (now - lastTouch < 300) e.preventDefault();
  lastTouch = now;
}, { passive: false });
```

Intended to block double-tap-zoom, but it *also* swallows the
synthesized click event on the second tap of any rapid double-tap.
In a game where FIRE is spammed, the second tap in a rapid burst
was being eaten ~1/3 of the time.

`touch-action: manipulation` blocks double-tap-zoom at the CSS layer
without touching click synthesis. The JS hack is not only unnecessary
but harmful. Removed.

## Prevention

- **Any engine/framework that sets `touch-action: none` globally is
  a red flag on iOS.** It kills click synthesis for non-button
  elements. The correct value is almost always `manipulation`
  (blocks pinch/double-tap-zoom) or `pan-y` (scrollable regions).
- **Never rely on default `-webkit-tap-highlight-color` on a dark
  UI.** Explicitly set it to `transparent` on body + every clickable
  element selector, at the top of your app's stylesheet.
- **Test tap-to-dismiss flows on a real iPad, not a Chrome device-
  emulator.** The `touch-action` / click-synthesis interaction does
  not reproduce in desktop browser simulation.
- **`preventDefault()` in `touchend` is almost never what you want
  for game controls.** It's a brute-force hack that breaks more than
  it fixes. Use `touch-action` CSS instead.

## Platform-specific regression test

For any future UI change touching dialogs, buttons, or touch
controls, the minimum manual test on iPad Safari:

1. Open Settings → tap backdrop → dialog closes.
2. Open Settings → tap a menu item → item activates.
3. Tap FIRE rapidly (6+ times in 1s) → every tap registers a shot.
4. Tap any button → no white/gray flash over the button surface.
5. Two-finger pinch on canvas → page does not zoom.
6. Double-tap on canvas → page does not zoom.

Items 1–2 failing → `touch-action` regression.
Item 3 failing → JS touch-event handler regression.
Item 4 failing → `-webkit-tap-highlight-color` regression.
Items 5–6 failing → lost `touch-action: manipulation`.
