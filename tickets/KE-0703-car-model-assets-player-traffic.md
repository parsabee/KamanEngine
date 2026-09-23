# KE-0703 — Car model assets (player + traffic)

Phase:         7
Priority:      P0
Status:        Todo
Integration:   New
Size:          M · A1
Time:          M
Risk:          Med
Depends on:    KE-0701, KE-0402      Blocks: KE-0706, KE-0708
Serves:        KR7.2

## Problem / Motivation
Replace the primitive box-cars with **authored, complex car models** imported through the glTF
pipeline (KE-0402). The player car and the traffic cars are real meshes, demonstrating the
asset→engine path a real game uses.

## Scope & Acceptance
- [ ] Author complex car meshes (in **Blender** or an equivalent DCC tool) and export `.glb`/`.gltf`;
      commit them under `games/playable-demo/assets/`. At least a player car + ≥1 traffic car variant.
- [ ] Import via `kaman-assets` (KE-0402); if the models exceed the current importer (multiple
      primitives/materials/nodes per mesh), extend the importer minimally and add a parse test.
- [ ] The player renders the player car; traffic (obstacle) entities render a traffic car; both
      load once and are shared by handle (KE-0402 cache).
- [ ] Reasonable scale/orientation facing the travel direction; wheels/body sit correctly on the road.

## Technical notes
- If no DCC tool is available in-environment, a committed **procedural glTF generator** (an `xtask`/
  example, like `kaman-assets`'s cube generators) that emits the `.glb` is acceptable — the deliverable
  is the committed asset + the import path, not the tool.
- Keep textures out of scope here (KE-0704 does the road; car textures can be a later polish ticket).

## Out of scope
- Road/building assets (KE-0704/KE-0706). Animation/skinning. LODs.

## Test gate
`cargo test -p kaman-assets` green (any importer extension covered by a parse test); `cargo run -p
playable-demo` shows the real car models; `--smoke` exits 0; clippy clean; firewall/de-brand hold.

## Doc gate
Document where the assets come from + how they're imported; `kaman-assets` README updated if the
importer changed.
