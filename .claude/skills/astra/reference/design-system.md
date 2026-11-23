# Mindify AI Frontier: design system cheat sheet

The source is `packages/tokens/tokens.json`, compiled to Tailwind v4 theme classes. Dark is primary, and Light
mirrors it (`<html data-theme="light">`). Monochrome first: green (`signal`) is a _signal_ (live, selected,
AI-made, focus), never decoration and never a button fill.

## Colour classes (`bg-*`, `text-*`, `border-*`)

| Token                                      | Use                                                                   |
| ------------------------------------------ | --------------------------------------------------------------------- |
| `surface`                                  | Page ground (#000)                                                    |
| `surface-alt`                              | Sidebars, asides, tab bars (#0A0A0A)                                  |
| `surface-raised`                           | Inputs, cards, panels (#111), always with a `line` edge               |
| `surface-overlay`                          | Menus, dialogs, popovers (#161616), with `shadow-lg`                  |
| `surface-sunken`                           | Code, logs, diffs (#050505)                                           |
| `line-subtle`                              | Row rules inside lists                                                |
| `line`                                     | Hairlines and panel edges                                             |
| `line-strong`                              | Input borders, idle dots                                              |
| `ink`                                      | Primary text                                                          |
| `ink-muted`                                | Secondary text; nothing lighter is allowed for text                   |
| `invert` / `on-invert`                     | Primary button fill and its text; human avatars                       |
| `signal` / `signal-soft`                   | Working, selected, active nav underline / tinted ground behind signal |
| `success`, `warning`, `danger` (+ `-soft`) | Merged and passed / needs you / errors and diff removals              |
| `scrim`                                    | Dialog backdrop                                                       |

## Type classes (`text-*`)

| Class                                      | Spec                                | Use                                                                  |
| ------------------------------------------ | ----------------------------------- | -------------------------------------------------------------------- |
| `display-lg` / `display-md` / `display-sm` | 56/60, 40/44, 28/32, 600, uppercase | One hero headline per screen (Welcome, Home, Merged, phone home)     |
| `title`                                    | 22/28, 600                          | Page title in product UI                                             |
| `heading`                                  | 16/24, 600                          | Panel/card heading, phone top-bar title                              |
| `lead`                                     | 20/30                               | Subhead under a display headline, `ink-muted`                        |
| `body` / `body-sm`                         | 16/26 / 14/22                       | Reading text / default product UI (the body default)                 |
| `label`                                    | 13/16, 600, 0.14em                  | Buttons, nav, tabs: **uppercase**                                    |
| `kicker`                                   | 12/16, 500, 0.24em                  | Eyebrows and section heads: **uppercase**, `ink-muted` (`<Eyebrow>`) |
| `spec` + `font-mono`                       | 11/16, 500, 0.08em                  | Tags, vendors, ids, meta lines: **uppercase** (`<Spec>`)             |
| `data` / `code` + `font-mono`              | 13/20                               | Branches, diff stats, logs (`<Data>`, `LogWell`, `DiffView`)         |
| `metric` / `metric-sm`                     | 72/72, 32/36                        | Big numbers, tabular figures                                         |

13px sentence-case text (status words, inline links) uses `text-[13px]` through `<Status>` and `inlineLinkClass`.

## Shape and spacing

- **Corners:** `rounded-0` by default (buttons, cards, avatars), `rounded-sm` (2px) for inputs and badges,
  `rounded-full` for dots only.
- **Spacing:** a 4px grid. Product screens use gaps of 4–32px; page insets are 24–48px; panels are padded 16–24px.
- **Control heights:** 32 (toolbar, cards), 40 (forms), 52 (hero CTAs), 44/48 on phones (touch targets ≥ 44).
- **Shadows:** flat. Separation is a hairline or a change of surface; only overlays get `shadow-lg`.
- **Focus:** global `box-shadow: var(--focus-ring)`, a 2px page-coloured gap plus 2px signal. Don't remove it.

## Primitives (`@astra/ui`)

| Need              | Use                                                                                                                                                                                            |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Buttons           | `Button` / `buttonClass(variant, size)`: `primary`, `outline`, `text`; sizes `sm` `md` `lg` `touch` `touchSm`                                                                                  |
| Status            | `Status tone=…` (dot + word), `SpecStatus` (mono "014 · WORKING"), `Dot`, `SectionHeading tone`                                                                                                |
| People and agents | `Avatar` (white square initials; 28/24/20), `AvatarStack`, `AgentGlyph` (outlined role letter)                                                                                                 |
| Text voices       | `Eyebrow`, `Spec`, `Data`, `TitleBlock`, `Badge` (signal-soft "REVIEWER'S PICK")                                                                                                               |
| Forms             | `Field` (label + `id` render prop), `TextInput`, `Select`, `TextArea`, `RadioCard`, `Radio`, `Fieldset`                                                                                        |
| Data              | `LogWell` (`verb  target` column), `DiffView` (+ green, − red), `formatDiffStats` (true minus sign), `KVList`, `Card`                                                                          |
| Chrome            | `WindowControls platform`, `TabBar variant`, `MobileTopBar variant`, `Segmented` (iOS), `UnderlineTabs` (Android)                                                                              |
| Icons             | `LogoMark`, `CheckIcon`, `ArrowRightIcon`, `ArrowLeftIcon`, `ChevronLeftIcon`, `ArrowUpIcon`, `CloseIcon`, `HomeIcon`, `ListIcon`, `ChipIcon`, `UsersIcon`. Stroke icons at 1.5px, never emoji |

## Accessibility (part of the design)

- Use real `<button>`, `<a href>`, `<label>`.
- Icon-only buttons need an `aria-label`.
- Decorative SVGs get `aria-hidden`.
- Text contrast is ≥ 4.5:1: only `ink` and `ink-muted` are used for text.
- Colours that must be told apart also differ in their word or shape (status dot + word).

## Copy voice

Short and declarative. Capitals for labels ("START MISSION →", "NEEDS YOU · 2"), sentence case for everything
else. No invented numbers or lorem ipsum: missing facts get honest empty states ("Nothing is waiting on you.").
