# Texture Rendering - Roadmap & TODO

This document tracks future improvements for GPUI's texture rendering capabilities.

## Current Status (2025-01-13)

The `textured-view` branch is now functional with:
- ✅ Async receiver wake mechanism (no more timer polling)
- ✅ Correct pixel format handling (BGRA passthrough)
- ✅ Working streaming mode with continuous render loop

## Nice-to-Have Improvements

### Short-term Polish

- [ ] **Remove hardcoded delays** - The BackgroundRenderer still has 10ms/20ms waits that could be eliminated or made configurable
- [ ] **Add cancellation support** - Currently the background thread runs until send fails; could add explicit cancellation token
- [ ] **Start/stop controls for streaming** - Add pause/resume API for streaming mode
- [ ] **Frame rate limiting on main thread** - Handle backpressure when main thread can't keep up with frames

### Medium-term Enhancements

- [ ] **Configurable frame timing** - Let users adjust delays for their use case
- [ ] **Frame dropping strategy** - When backpressured, skip frames intelligently
- [ ] **Error propagation** - Better error reporting from background thread to main thread
- [ ] **Resource cleanup** - Ensure GPU resources are properly released on drop

### Long-term Architectural

- [ ] **GPUI-level cross-thread notification API** - General-purpose mechanism for background threads to wake the main thread
- [ ] **Shared GPU context approach** - Single GPU context shared between main and background rendering (see Alternative Approach below)
- [ ] **Documentation** - Comprehensive guide for async rendering patterns in GPUI

---

## Alternative Approach: `gpui-render-to-texture` Branch

There is an alternative implementation in the `gpui-render-to-texture` branch that takes a different architectural approach:

### Key Difference

Instead of spawning a separate `Application::textured()` instance with its own GPU context, the `gpui-render-to-texture` approach **shares the main application's GPU context** for offscreen rendering.

### Advantages

- Single GPU context (less memory, simpler resource management)
- No inter-process/inter-thread pixel copying
- Can use GPU→GPU texture copies instead of readback
- More natural integration with GPUI's existing rendering pipeline
- Potentially better performance

### Current Status

The branch exists but has some unresolved bugs. It was set aside when the `textured-view` approach started working, but it represents the **preferred long-term direction**.

### Next Steps

1. Revisit the `gpui-render-to-texture` branch
2. Document the bugs/difficulties encountered
3. Consult with GPUI maintainers on the best architectural direction
4. Consider whether this should be the primary approach going forward

### Questions for GPUI Team

- Is sharing the GPU context the right architectural direction?
- Are there concerns about rendering to textures from the main render loop?
- How should offscreen rendering integrate with the existing frame/present cycle?
- Any plans for official offscreen/texture rendering APIs?

---

## References

- `gpui/src/textured_view.rs` - Current TexturedView implementation
- `gpui/src/platform/linux/textured_surface/` - Headless rendering backend
- `gpui/research/texture_rendering_lifecycle_analysis.md` - Detailed analysis
- `gpui/research/gpui_async_rendering_investigation.md` - Initial investigation
- Branch: `textured-view` - Current working implementation
- Branch: `gpui-render-to-texture` - Alternative shared-context approach (needs work)