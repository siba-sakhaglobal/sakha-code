---
name: design-md
description: Author and apply DESIGN.md design-system files (design tokens in YAML frontmatter + prose rationale); use when creating or restyling UI so visual identity is consistent across sessions and tools.
---

# DESIGN.md: Design System Format

DESIGN.md is a self-contained, plain-text representation of a design system. It captures the visual identity of a brand/product — colors, typography, spacing, shapes, elevation, and component styling — so that identity stays consistent across design sessions and across different AI agents/tools (compatible with Claude Code, Gemini CLI, and other DESIGN.md-aware tooling). It is a living source of truth that both humans and AI can read and refine.

## File Structure

A `DESIGN.md` file has two parts:

1. **YAML frontmatter** (optional but recommended) — machine-readable design tokens, delimited by a line containing exactly `---` at the start and another exactly `---` at the end.
2. **Markdown body** — human-readable sections giving design rationale and guidance. Prose may use descriptive names ("Midnight Forest Green") that map to systematic token names (`primary`); the tokens are normative, the prose gives context for applying them.

## Design Tokens (Frontmatter Schema)

```yaml
---
version: alpha          # optional
name: <design system name>
description: <string>   # optional
colors:
  <token-name>: <Color>
typography:
  <token-name>: <Typography>
rounded:
  <scale-level>: <Dimension>
spacing:
  <scale-level>: <Dimension | number>
components:
  <component-name>:
    <token-name>: <string | token reference>
---
```

- **Color**: any valid CSS color string — hex (`#RGB`, `#RRGGBB`, `#RRGGBBAA`), named (`cornflowerblue`), functional (`rgb()`, `hsl()`, `hwb()`), wide-gamut (`oklch()`, `oklab()`, `lch()`, `lab()`), or `color-mix()`. Prefer hex for simplicity/tooling support. All colors are converted to sRGB internally for WCAG contrast checking.
- **Typography**: an object with `fontFamily` (string), `fontSize` (Dimension), `fontWeight` (number, e.g. `400`/`700`), `lineHeight` (Dimension or unitless multiplier, e.g. `1.5`), `letterSpacing` (Dimension), and optionally `fontFeature`/`fontVariation` (raw CSS feature/variation-settings strings).
- **Dimension**: a string with unit suffix — `px`, `em`, or `rem`.
- **Token references**: `{path.to.token}` — must point to a primitive value (e.g. `{colors.primary-60}`), except within `components`, where references to composite values like `{typography.label-md}` are allowed.
- `<scale-level>` is a free-form key (commonly `xs`, `sm`, `md`, `lg`, `xl`, `full`).

Recommended (non-normative) token names: colors — `primary`, `secondary`, `tertiary`, `neutral`, `surface`, `on-surface`, `error`; typography — `headline-display`, `headline-lg/md`, `body-lg/md/sm`, `label-lg/md/sm`; rounded — `none`, `sm`, `md`, `lg`, `xl`, `full`.

## Body Sections (in order; omit any that don't apply)

1. **Overview** (aka "Brand & Style") — brand personality, target audience, emotional tone (playful vs. professional, dense vs. spacious). Foundational context for decisions no token covers.
2. **Colors** — palette description in prose (primary/secondary/tertiary/neutral roles), backed by the `colors` token map. At minimum `primary` must be defined.
3. **Typography** — typeface strategy and level roles (headline/display/body/label/caption), backed by the `typography` token map. Most systems define 9-15 levels.
4. **Layout** (aka "Layout & Spacing") — grid model, spacing scale/rhythm, containment patterns, backed by the `spacing` token map.
5. **Elevation & Depth** — how hierarchy is conveyed: shadow-based depth, or flat-design alternatives (borders, tonal contrast).
6. **Shapes** — corner-radius language and geometric character, backed by the `rounded` token map.
7. **Components** — per-component styling (buttons, chips, lists, tooltips, checkboxes, radios, inputs), backed by the `components` token map. Each component's properties are themselves tokens: `backgroundColor`, `textColor`, `typography`, `rounded`, `padding`, `size`, `height`, `width`. State variants use suffixed keys (`button-primary-hover`, `button-primary-active`).
8. **Do's and Don'ts** — concrete guardrails and common pitfalls as a bullet list.

Use `##` headings for every section; an optional `#` title is allowed but not parsed as a section. A duplicate section heading is invalid (reject/flag the file). Unknown headings, token names, or component properties should be preserved/accepted, not treated as errors — DESIGN.md consumers are forward-tolerant.

## Workflow

1. **Check for an existing DESIGN.md** in the project root (or wherever the project keeps design docs) before writing or restyling any UI.
   - **If found**: read it fully. Apply its tokens *exactly* (exact hex values, exact type scale, exact spacing/radius scale) — do not eyeball or approximate. Use the prose sections to resolve any ambiguity a token alone doesn't cover (e.g. which color to use for a given emphasis level).
   - **If absent**, and the task involves meaningfully creating or restyling a UI (not a one-line tweak): author a `DESIGN.md` first, covering at least Overview, Colors, and Typography (add other sections as the project's surface area grows), then implement the UI from it. This keeps future sessions/agents consistent with today's choices instead of re-deriving style ad hoc.
2. **Implement UI from the tokens**: map token values directly onto the target stack's styling mechanism (CSS custom properties, Tailwind theme config, styled-components theme, Figma variables, etc.) rather than hardcoding literal values scattered through components, so the DESIGN.md file remains the single source of truth.
3. **Validate accessibility**: check WCAG AA contrast (4.5:1 for normal text, 3:1 for large text/UI components) between foreground/background token pairs actually used together. Adjust token values (not just the specific instance) if a pairing fails, so the fix propagates everywhere that pairing is used.
4. **Keep DESIGN.md in sync**: if a UI change requires a new token or a deviation from an existing one, update DESIGN.md in the same change — it must stay a living, accurate source of truth, not a stale snapshot.

## Example Frontmatter

```yaml
---
version: alpha
name: Daylight Prestige
colors:
  primary: "#1A1C1E"
  secondary: "#6C7278"
  tertiary: "#B8422E"
  neutral: "#F7F5F2"
typography:
  h1:
    fontFamily: Public Sans
    fontSize: 48px
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: -0.02em
  body-md:
    fontFamily: Public Sans
    fontSize: 16px
    fontWeight: 400
    lineHeight: 1.6
spacing:
  base: 16px
  sm: 8px
  md: 16px
  lg: 32px
rounded:
  sm: 4px
  md: 8px
  full: 9999px
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.neutral}"
    rounded: "{rounded.md}"
    padding: 12px
---
```
