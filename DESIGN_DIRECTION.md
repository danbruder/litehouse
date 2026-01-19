# Litehouse Visual Direction — Warm Infrastructure

Litehouse is a server management platform for self-hosting SQLite apps (single-host containers + Litestream). The market is saturated with terminal/mono aesthetics; this direction intentionally avoids “CLI cosplay” and instead communicates **calm reliability** and **human-friendly infrastructure**.

## Brand positioning

**Litehouse feels like:**
- Calm, dependable, quietly powerful
- Local-first, single-host, “steady by default”
- Self-hosting without anxiety

**Litehouse is not:**
- Hacker-terminal cosplay
- Cyberpunk neon
- Black-on-green nostalgia UI

Core metaphor:
> “Modern architecture + cozy infrastructure”  
(“infrastructure as shelter”)

---

## Visual direction: Warm Infrastructure

A modern product UI with warm neutrals and a guiding-light brand accent (lighthouse amber). Soft geometry, gentle motion, and card-first layouts make complex systems feel manageable.

---

## Color system

### Base neutrals (calm, not terminal)
- Background: `#F7F6F3` (warm off-white)
- Surface: `#FFFFFF`
- Primary Text: `#2B2B2B`
- Secondary Text: `#6B6B6B`
- Border: `#E3E1DC`

Guidance:
- Avoid pure black and pure white as defaults.
- Prefer warm neutrals for less “screaming” contrast and more trust.

### Primary brand: Lighthouse Amber
- Amber: `#F2A900`
- Deep Amber: `#C98200`
- Soft Glow: `#FFE6B3`

Use sparingly for:
- Primary CTA emphasis
- “Everything is OK” moments (deploy success, synced state)
- Accent highlights, focus rings, small status dots

### Supporting cool counterbalance
- Slate Blue: `#4A6FA5`
- Mist Blue: `#E6EEF7`

Use for:
- Secondary actions
- Charts & info surfaces
- Settings sections
- Background gradients/sections

### Status colors (muted, grown-up)
- Success (sage): `#6FAE7B`
- Warning (muted gold): `#E0B15C`
- Error (dusty red): `#C96B6B`

Guidance:
- Nothing should scream.
- Favor soft fills with clear text rather than intense neon alerts.

---

## Typography

### Headings: humanist sans
Recommended:
- Manrope (excellent for product UI) - LET's USE THIS ONE
- Inter (safe default)
- Source Sans 3
- IBM Plex Sans (subtle engineering cred)

### Body
- Same family as headings, lighter weight
- Favor readability and low fatigue

### Monospace (purposeful, minimal)
- JetBrains Mono or IBM Plex Mono
Use only for:
- file paths
- logs
- sqlite filenames
- config snippets

Principle:
> Mono is a tool, not the personality.

---

## Iconography

### Style: soft-edged technical icons
Avoid:
- Sharp outline-only icon sets as the core identity
- Terminal prompts / hacker glyphs

Prefer:
- Rounded corners
- Slightly thicker strokes
- Filled or semi-filled variants for emphasis

Metaphor ideas (not literal server racks):
- Apps → rooms/tiles
- Databases → layers/stacked stones
- Backups → light beams/reflections
- Deploys → doors opening
- Logs → scrolls/pages

---

## Layout & UI patterns

### Card-first, not table-first
Apps should feel “manageable”:
- App card: name, status dot, last sync, sqlite size, host
- Tables only when users need sorting/filtering at scale

### Soft containers
- 12–16px radius
- Light shadow
- Minimal hard dividers

Suggested shadow:
- `0 1px 3px rgba(0,0,0,0.06)`

---

## Motion

Make motion slow and intentional:
- Sync running: gentle pulse
- Backup running: slow sweep
- Deploy complete: warm glow

Avoid:
- jittery spinners
- frantic “urgent” motion language

---

## Artwork & illustration direction

### Theme: infrastructure as shelter
Illustration subjects:
- Small house with warm lights
- Lighthouse beam on a calm shore
- Greenhouse-like structure with glowing nodes
- Rooms labeled “apps”, “data”, “backups” (abstract)

Style:
- Flat shapes + subtle gradients
- No characters required
- Muted palette; warm highlights for “working” states

Hero concept:
- A single glowing structure (Litehouse) with connected tiles (apps)
- One stable foundation (SQLite)
- No clouds/globes

Tagline ideas:
- “Self-hosting that feels steady.”
- “Your apps. Your host. Calm backups.”

---

## Logo direction

Avoid:
- Hexagons
- Database cylinder clichés
- Terminal prompt motifs

Explore:
- Lighthouse beam mark
- House outline + lit window
- Simple geometric symbol (one-color friendly)

Must work well as:
- favicon
- CLI banner mark
- tray icon
- docker image badge

---

## Voice & microcopy

Tone: calm, confident, pastoral.

Prefer:
- “Backups are running.”
- “Synced 2 minutes ago.”
- “Your data is safe.”
- “Nothing to fix.”

Avoid:
- “Deploy failed!!!”
- “Uh oh”
- “🔥” style urgency

