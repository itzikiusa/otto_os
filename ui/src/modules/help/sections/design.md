---
id: design
title: Design Hall
group: Build
route: design
summary: One versioned library for every design studio (frames, graphics, sites, 3D, whiteboards and brand kits), with agents that draft, refine and cite earlier work.
---

## What it's for

Design Hall is where your team's visual work lives. Every design is an artifact in a studio, every save is a version, and designs link to each other, to brand kits and to Product stories. Agents can generate a design, refine it, suggest variants and check accessibility. Every agent result is a new version that a person reviews and approves.

## Getting started

1. Open **Design Hall** in the sidebar.
2. In the lobby, describe what you want in the prompt, pick a studio, and choose how many variants (1 to 4). Tick **Use references** to give the agent your team's closest past designs, or **Draft only** to skip generation.
3. The design opens on its **Otto** tab while the agent works. When it finishes, the result is a new version.
4. Edit the design, then choose **Save** (`⌘S`). Use **Compare** to see what changed between versions.
5. When it's ready, set the status to **Approved**. Designs that follow the approved version update to it.

## Everything it can do

**The lobby**
- A prompt to generate a design, the seven studios, recently edited designs, projects, designs linked to Product stories, and a panel of what Otto learned and recent agent activity.
- Search designs, stories and references. **Spatial** shows your projects as a gallery (beta).
- ⌘K commands: **New design…**, **New design project…**, **Import design file…** (PNG, SVG, GLB, HTML, Excalidraw and more), **Open Brand Kit** and **Open what Otto learned**.

**Studios**
- **Frames**: UI screens as HTML with a live device preview.
- **Graphics**: HTML boards, SVGs and imported images.
- **Site Studio**: websites built from sections, with brand themes.
- **3D Studio**: scenes, models and text-to-3D.
- **Whiteboard**: Excalidraw, Mermaid and D2 diagrams (the Canvas page).
- **Brand Kit**: the colours, type and logos every studio uses.
- **Spatial Hall**: a showcase of your projects (planned; a preview of the room layout is available).

**A design**
- A breadcrumb to its project, a status menu (draft, review, approved, shipped, archived), and a version strip. Only a person can approve.
- **Compare** shows two versions side by side or as text changes. **Restore** saves the older version as a new one; history never goes back.
- ⋯ menu: **Save named version…**, **Discard edits…**, **Rename…**, **Move to project…**, **Make an editable copy**, **Download**, open in a new window, and **Archive…**.
- **Links** shows what the design uses and where it is used, and lets you add or remove links. **References** searches the library: add a design as a reference, start from it, or compare with it.
- Designs imported from Product or Canvas open read-only here, with a link back and **Make an editable copy**.
- If someone else saves while you edit, you choose between saving yours on top or loading theirs.

**The Otto tab (every studio)**
- Pick an agent and model, then ask for a change. Quick actions: **More engaging**, **Check accessibility**, **On-brand check**, **Fix mobile**, **Real copy from story** and **3 variants**. The two checks report first and you choose what to fix.
- Modes: generate a new design, refine the current one, critique it (findings only, no edits), or fix accessibility.
- The agent cites earlier designs it borrowed from. Otto checks each citation and flags any it can't verify.
- Variants arrive in a tray where you can **Apply**, **Compare** or reject them with a reason.

**3D Studio**
- A hierarchy (primitives, lights, groups, GLB import), environments, saved named views and a scene card showing the brand kit.
- A viewport with a move, rotate and scale gizmo, a view cube, named states that animate between each other, and a stats pill.
- An inspector for transform, corner radius, material presets, brand colours, material sliders, notes and where the scene is used.
- **✨ Generate ▾**: a blockout from a prompt, text to 3D model, image to 3D model, or refine in Blender through the Blender MCP. Each runs one agent turn and asks before any Blender tool runs.
- **Turntable**, **Play**, **Source** (the scene JSON), and **Export ▾** as GLB, USDZ, PNG snapshot, PNG turntable or scene JSON. **Optimize for web** compresses a GLB on your Mac.

**Site Studio**
- Desktop, tablet and mobile widths, zoom, motion preview, undo and redo.
- Pages, layers and a library of 27 section blocks, plus sections reused from your other sites.
- Click to select, double-click text to edit it in place. A floating toolbar offers **Ask Otto**, **Variants**, move, duplicate and hide on mobile.
- An inspector for content, style (brand colours flagged when off-brand), responsive options, motion and live contrast checks.
- **Preview** and **Publish ▾**: export a static site as a .zip, or preview it locally. Nothing is sent outside your Mac.

**Brand Kit**
- Colours with live contrast badges, typography, spacing and radius, logos, imagery and voice guidelines, and a list of every design that uses the kit.
- Before you save, Otto shows which designs a token change will reach and when each one updates.
- **Approve** a version separately. **Export ▾** as CSS variables, a Tailwind `@theme` block or DTCG tokens (JSON).
- Start a new kit from the Vivid, Editorial or Minimal starter.

**Whiteboard (Canvas)**
- Choose an Excalidraw board, a Mermaid diagram or a D2 diagram when you create a canvas.
- Describe what you want in **Ask AI**; the agent edits the diagram and the board redraws as it changes.
- Edit Mermaid and D2 source in the **Code** panel. D2 has a hand-drawn sketch style.
- Zoom, fit to screen, **Download PNG**, **Download SVG** and **Copy source**.
- Rename, duplicate, delete or move a canvas to a section from its row’s **⋯** menu (or right-click; `F2` renames). A canvas can also be attached to an agent session.

**What Otto learned**
- Otto notices choices your team repeats, such as a variant direction you keep picking or a fix you keep making after agent drafts, and proposes a team rule.
- Rules are only proposed. A person approves, rejects or rolls back each one, and approved rules are included in every design turn.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `⌘S` | Save the design or brand kit |
| `⌘Z` | Undo in Site Studio |
| `⇧⌘Z` | Redo in Site Studio |
| `Esc` | Deselect in Site Studio |
| `Enter` or `⌘Enter` | Keep a text edit on the Site Studio canvas |
| `Esc` | Cancel a text edit on the Site Studio canvas |
| `W` / `E` / `R` | Move, rotate or scale in the 3D viewport |
| `F` | Frame the selection in the 3D viewport |
| `⌘D` | Duplicate the selected 3D object |
| `Delete` or `⌫` | Delete the selected 3D object |
| `Esc` | Deselect in the 3D viewport |
| `F2` | Rename the selected item in the 3D hierarchy |
| `←` / `→` | Move between versions in the version strip |
| `Esc` | Clear the lobby search |
| `Enter` | Send a message to the Whiteboard assistant |
| `⇧Enter` | New line in the Whiteboard assistant |

## Tips and limits

- Access needs the **Design Hall** feature: View to read, Edit to change, Admin for import and pruning. The Whiteboard page (Canvas) uses the **Canvas** feature.
- One agent turn runs per design at a time, for up to 20 minutes. A variants run makes at most 4 variants.
- A design can be up to 25 MB per version. Images, GLB and PDF files can't be edited by an agent.
- Agents see the last thumbnail the app stored, not a fresh render.
- Text and image to 3D build models from primitives through the agent. Cloud 3D providers are listed but disabled.
- Frames and Graphics currently edit HTML or SVG as source; their dedicated canvases are planned.
- Changes made in Product's Design tab or on the Canvas page reach Design Hall at the next import, not live.

## Related

- [Product](#/walkthroughs/product)
- [Vault](#/walkthroughs/vault)
- [Agents](#/walkthroughs/agents)
- [Swarm](#/walkthroughs/swarm)
- [MCP Control Plane](#/walkthroughs/mcp)
