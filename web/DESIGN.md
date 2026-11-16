---
name: Runlark UI Kit
description: Storybook UI kit using our Mantine v9 adapters, adapted from the approved Pencil GO73T and W1FN0 references.
colors:
  ink: "#20232B"
  ground: "#F3F4F6"
  paper: "#FFFFFF"
  muted: "#646B79"
  breadcrumb: "#596475"
  rule: "#DFE2E8"
  control-border: "#D4D8E0"
  card-border: "#DCE1E9"
  sidebar: "#20232B"
  sidebar-surface: "#2B2F39"
  sidebar-border: "#30343E"
  sidebar-text: "#E0E4EB"
  sidebar-muted: "#AEB7C7"
  accent: "#EDB830"
  attention-wash: "#FBF2D8"
  decision-paper: "#FFFCF3"
  decision-border: "#C6A143"
  success: "#2E7D4F"
  success-text: "#286D45"
  success-wash: "#E3F1E8"
  danger: "#C0392B"
  danger-text: "#A12D22"
  danger-wash: "#F8E4E1"
  unknown: "#5B6475"
  unknown-wash: "#EBEEF3"
typography:
  page-heading:
    fontFamily: "Manrope, system-ui, sans-serif"
    fontSize: "1.875rem"
    fontWeight: 800
    lineHeight: 1.25
    letterSpacing: "-0.044rem"
  mobile-heading:
    fontFamily: "Manrope, system-ui, sans-serif"
    fontSize: "1.5rem"
    fontWeight: 800
    lineHeight: 1.25
    letterSpacing: "-0.03em"
  column-heading:
    fontFamily: "Manrope, system-ui, sans-serif"
    fontSize: "1rem"
    fontWeight: 600
    lineHeight: 1.4
  card-title:
    fontFamily: "Manrope, system-ui, sans-serif"
    fontSize: "0.9375rem"
    fontWeight: 600
    lineHeight: 1.4
  ui:
    fontFamily: "Manrope, system-ui, sans-serif"
    fontSize: "0.8125rem"
    fontWeight: 400
    lineHeight: 1.5
  meta:
    fontFamily: "Manrope, system-ui, sans-serif"
    fontSize: "0.75rem"
    fontWeight: 400
    lineHeight: 1.4
  nav-label:
    fontFamily: "Manrope, system-ui, sans-serif"
    fontSize: "0.6875rem"
    fontWeight: 700
    lineHeight: 1.3
    letterSpacing: "0.08em"
  figure:
    fontFamily: "Roboto Mono, ui-monospace, monospace"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.6
rounded:
  control: "0.375rem"
  card: "0.375rem"
  badge: "0.3125rem"
  segment: "0.25rem"
spacing:
  "4": "0.25rem"
  "6": "0.375rem"
  "8": "0.5rem"
  "10": "0.625rem"
  "12": "0.75rem"
  "14": "0.875rem"
  "16": "1rem"
  "20": "1.25rem"
  "24": "1.5rem"
  "28": "1.75rem"
components:
  sidebar:
    backgroundColor: "{colors.sidebar}"
    textColor: "{colors.sidebar-text}"
    width: "14rem"
    padding: "1.25rem 0.875rem 1rem"
  breadcrumb-bar:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.breadcrumb}"
    minHeight: "2.875rem"
    padding: "0 1.75rem"
  page-heading:
    textColor: "{colors.ink}"
    typography: "{typography.page-heading}"
  action-button:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "0.5rem 0.875rem"
  status-badge:
    rounded: "{rounded.badge}"
    padding: "0.125rem 0.5rem"
  board-column:
    backgroundColor: "{colors.ground}"
    rounded: "{rounded.card}"
    padding: "0.75rem"
  board-card:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    rounded: "{rounded.card}"
    padding: "0.875rem"
  notice-strip:
    backgroundColor: "{colors.attention-wash}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
  filter-toolbar:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    minHeight: "3.25rem"
    padding: "0 1.5rem"
---

# Design System: Runlark UI Kit

## Overview

**Creative North Star: "The Quiet Workbench".**

Runlark is an operating surface. A dark navigation rail frames a bright, dense work area; content sits on white sheets over a cool grey ground. The approved Pencil references `GO73T` (desktop) and `W1FN0` (mobile) set the composition, density and visual language. Storybook renders that system rather than a picture of it.

Colour is a signal, not an atmosphere: golden accent marks an action, an active destination or a decision the viewer can take; green, red and slate name recorded outcomes; everything else is ink, rule and paper. The dark sidebar is a region inside a light product, not a dark theme. No second theme switcher exists.

**Key Characteristics:**

- Dark 224px navigation rail against a light, quiet working surface.
- Manrope for interface and headings, Roboto Mono for figures and identifiers.
- Six-pixel controls and cards, five-pixel badges, four-pixel segmented edges.
- Colour appears only for action, attention and semantic state.
- Unknown stays visible as slate and is never folded into a known total.
- Dense desktop board; one active stage with previous/next controls on mobile.

## Colors

### Primary

- **Accent Goldenrod** (`accent`): primary action, active navigation destination, focus on dark surfaces, and a decision the viewer can act on. It is never a generic decoration.
- **Attention Wash** (`attention-wash`): the strip that asks for a person's attention and the hover/selected wash behind open decisions.

### Neutral

- **Ink** (`ink`): primary text, icons and the light-surface focus outline.
- **Ground** (`ground`): page and board background.
- **Paper** (`paper`): cards, sheets, inputs, header and footer surfaces.
- **Muted** (`muted`): secondary text, help and metadata outside breadcrumbs.
- **Breadcrumb** (`breadcrumb`): breadcrumb strip text only.
- **Rule**, **Control Border**, **Card Border**: dividers, input/control outlines and card outlines respectively.
- **Sidebar**, **Sidebar Surface**, **Sidebar Border**: the navigation region and its raised rows.
- **Sidebar Text**, **Sidebar Muted**: readable text and group labels on the dark rail.

### Semantic

- **Success**, **Success Text**, **Success Wash**: accepted work, a recorded pass and a completed action. Always accompanied by text.
- **Danger**, **Danger Text**, **Danger Wash**: failure, invalid input, changes requested and destructive confirmation.
- **Unknown**, **Unknown Wash**: an outcome, evidence or charge that is not known. Never ordinary disabled state.
- **Decision Paper**, **Decision Border**: a card that carries the viewer's own pending decision.

### Named Rules

**The One Meaning Rule.** Goldenrod is action or attention; green is recorded success; red is failure or required repair; slate is unknown. A colour never borrows another meaning, and colour is never the only carrier of a state.

**The Unknown Stays Visible Rule.** An unknown outcome, evidence state or cost is shown in slate with explicit text. It is not rendered as zero, not folded into a known total and not styled as a pass or as ordinary disabled.

**The Dark Region Rule.** The sidebar is a navigation region in a light product. Do not extend its dark surfaces into the workspace or interpret it as an invitation to build a dark theme.

## Typography

**UI font:** Manrope, self-hosted from Fontsource in weights 400, 600, 700 and 800, with Latin and Cyrillic coverage. Fallback: `system-ui, sans-serif`.

**Figure font:** Roboto Mono, self-hosted in 400 and 600. Fallback: `ui-monospace, monospace`.

**Character:** a compact geometric sans for reading and hierarchy, and a mechanical mono wherever values are compared, copied or aligned. Figures use tabular numerals.

### Hierarchy

- **Page heading** (800, 1.875rem desktop / 1.5rem mobile, 1.25): one `h1` per page; letter spacing −0.044rem on desktop and −0.03em on mobile.
- **Column heading** (600, 1rem): board column and section subheading.
- **Card title** (600, 0.9375rem, 1.4): work or component title inside a card.
- **Interface text** (400, 0.8125rem): controls, navigation rows, body copy and help.
- **Metadata, badge, identifier** (400–600, 0.75rem): metadata, badges and identifiers; identifiers use the mono face.
- **Navigation group label** (700, 0.6875rem, tracked 0.08em): the small label above a navigation group.
- **Figure** (mono, 0.875rem, tabular): references, counts, money and durations.

On touch viewports, input text is at least 1rem to avoid browser auto-zoom. Card typography keeps the desktop sizes above; the larger input is an accessibility adaptation, not a redesign.

### Named Rule

**The Figure Rule.** Money, references, counts and durations are set in Roboto Mono with tabular numerals, and right-align when they form a column. Never mix prose and figures in the same numeric column.

## Layout

Desktop starts at 48em. The shell is a 14rem (224px) dark rail and a fluid workspace. The workspace is a grid of a 2.875rem (46px) breadcrumb strip, the main region and the sample-data footer. The rail keeps its middle navigation independently scrollable and its footer pinned.

Page content uses a 24px gutter (28px for the breadcrumb strip) and a 20px top pad. A page heading has a minimum height of 6rem, not a clipping box: long Russian or English text grows it. The filter strip is at least 52px and the attention strip at least 50px. These are minimums, never fixed heights that hide content.

The board occupies the available height below the header and toolbar. `BoardSurface` owns two-axis scrolling with `min-width: 0`; the page itself never scrolls horizontally. Columns are at least 256px wide with a 12px gap. From 768 to 1279px they keep 256px and scroll horizontally; at 1280px and above a `minmax(256px, 1fr)` grid distributes the extra width. At the 1600px reference, five columns resolve to 256px.

Mobile is below 48em. A 60px header carries the brand and menu, content uses a 16px gutter, and a 64px bottom navigation plus safe-area inset closes the viewport. The sidebar becomes a focus-trapped drawer, never an offscreen focusable rail. The board shows one active stage behind previous/select/next controls; the other stages are reached through the same controlled menu as desktop. The page has no horizontal overflow.

**The Fluid Workspace Rule.** Navigation keeps its fixed rail; only the board surface may scroll horizontally. Content that does not fit wraps or grows rather than being clipped.

## Elevation & Depth

Flat. Depth comes from the ground/paper contrast, one-pixel rules and the dark rail. There are no drop shadows, glass panels or blurred overlays. Menus, modals and drawers use Mantine's overlay and a solid paper surface; the overlay is the only dimmed layer.

Focus is structural: a 2px ink outline at 2px offset on light surfaces, and a 2px accent outline on the dark rail. Disabled controls keep a readable explanation; opacity alone is not the only signal.

## Shapes

Controls and cards use 6px corners; badges use 5px; segmented control edges use 4px. Avoid pill-shaped statuses unless the underlying control is genuinely a pill. Borders are one pixel in `control-border` or `card-border`. The decision card combines `decision-paper` with a `decision-border` outline; success, danger and unknown surfaces use their wash plus a matching border and text.

## Components

### Navigation rail

- Dark `sidebar` surface, 14rem wide, 224px at the GO73T reference.
- Official reverse horizontal lockup at 192px. On mobile the micro mark is 34px beside the Runlark name.
- Group labels are tracked 11px `sidebar-muted`; rows are 13px/600 `sidebar-text`.
- The active destination uses `sidebar-surface` plus a 2px accent edge and `aria-current="page"`.
- The footer holds the profile/identity entry and the RU/EN switch once those surfaces exist.

### Breadcrumb strip and page heading

- 46px minimum, paper background, `breadcrumb` text, 24px desktop gutter and 28px for the strip.
- Breadcrumb anchors are underlined; the last item is plain text with `aria-current="page"`.
- The heading block owns one `h1`, a wrapping action area and optional metadata. It is not a hero.

### Toolbar, filters and notice

- The filter toolbar is a form-neutral region: search, filters and actions wrap independently; actions follow filters.
- The attention strip uses `attention-wash`, ink text and one action, and is at least 50px tall.
- A notice is static by default. `role="alert"` is reserved for a current error, not every status card.

### Cards

- Paper surface, 1px `card-border`, 6px radius, 14px padding and 10px internal gap.
- The card title is a 15px/600 heading; metadata and identifiers are mono 12px.
- A decision the viewer can act on uses `decision-paper` and `decision-border`. Someone else's pending decision is neutral, with the reason in text.
- The whole card is never a button. The title is a real link or button; the drag handle is a separate pointer region, and the keyboard path is the "More / Move" menu.

### Status and semantic badges

- Badge label is text; colour supports it. Neutral, attention, success, danger and unknown tones.
- Unknown is slate with an explicit label. Disabled is not unknown, and unknown is not an empty state.
- Long labels wrap inside the card; they are not truncated to protect a colour chip.

### Board columns

- Section heading (16px/600) with a count and optional actions; column content is an ordered list supplied by the caller.
- Empty columns keep their heading and explain emptiness. The empty state is different from "no search results".
- The drop region has a minimum height of 88px. Native dragging starts from the separate handle. Live-region announcements describe the current target column and position. Allowed drops request a move from the host without optimistic sorting. The accessible menu offers the same requests.
- During a drag, the source collapses to a 64px row with a muted dashed border and a 20px map pin. An allowed destination opens a 64px row in the column with an attention wash, a golden outline and a 20px downward arrow. The rows span the column content width. Adjacent cards move into place over 120 ms with ease-out. Reduced motion makes the shift immediate.
- A single live copy preserves the source card's dimensions and pivots around the grab point. It settles into a shallow perspective tilt with a −5° in-plane turn in 160 ms. Two paper-white, ink-outlined wings with a golden root accent open once in 220 ms, clear of the card content. A soft static shadow gives the raised card depth. The copy follows the pointer directly.
- The flight copy disappears when the gesture ends. With reduced motion it keeps its wings open in a flat, static pose with no shadow.

### Buttons, menus and dialogs

- Primary action is accent with ink text; secondary is paper with a control border; danger uses the danger tone. One primary action per form or dialog.
- Menus are controlled and support keyboard focus, Escape and disabled items with a visible reason.
- Confirmation dialogs trap focus, return focus to the trigger, close on Escape/outside as cancel, and block closing while a command is pending.
- Destructive confirmations start focus on Cancel.

### Forms

- Inputs use paper, a 1px control border, 6px radius and the UI font. Touch inputs are at least 16px.
- An error is a red border plus a readable problem line; help text stays visible.
- No placeholder is the only label.

## Accessibility and states

- Minimum contrast: 4.5:1 for text and 3:1 for large text and UI boundaries.
- Focus is always visible; the sidebar uses accent focus, the light workspace uses ink.
- Hover, focus, active, disabled, loading and error states are visually distinct without relying on colour alone.
- Keyboard paths are real: menus, dialogs, drawers and filters are usable without a pointer. Drag has an equivalent menu workflow.
- Reduced motion is respected; no animation is required to understand a state.
- Announcements are localised and never claim success before the host commits the change.

## Do's and Don'ts

### Do

- **Do** keep the dark rail as a region and the workspace light.
- **Do** use the accent for action, active destination and the viewer's own pending decision only.
- **Do** show unknown values as slate with explicit text, separate from known totals.
- **Do** keep figures, references and money in tabular Roboto Mono.
- **Do** allow long Russian and English labels to wrap; minimum heights are not clipping heights.
- **Do** keep focus visible and give every disabled action a readable reason.

### Don't

- **Don't** bring back the old golden top band, Public Sans or the previous card-grid prohibition; the owner approved GO73T/W1FN0 as the visual reference.
- **Don't** turn the sidebar into a dark theme or add a theme switcher.
- **Don't** use colour as the only signal for success, danger, unknown or attention.
- **Don't** style unknown as ordinary disabled or render it as zero.
- **Don't** add drop shadows, glass, gradients, surprise motion or a marketing hero to an operating screen.
- **Don't** hide the reason an action is unavailable, or present a Storybook fixture action as a shipped product command.

## Implementation boundary

Examples live only in Storybook (`web/stories/`), outside reusable production modules.
The product Task→PR client lives in `web/src/app`, `pages` and `modules/task-flow`; it uses real
API projections and authorized commands. There is no second runtime catalogue. Consumers use our
readonly props/slots contracts in `common/ui/<component>`; only those adapters import Mantine.
The reference board assembles this system with nine local works, five stages and two own decisions.
Its move preview can change positions only: execution, acceptance and known/held/unknown charges
remain independent. Demo/scenario controls are opt-in Storybook controls, not part of the visual reference.
