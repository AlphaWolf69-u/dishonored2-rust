# Windows fullscreen style

Fullscreen removes the overlapped window styles. Mark top-level fullscreen
windows as `WS_POPUP`, so the topmost HWND remains eligible for a DXGI swap
chain. Without that flag, normal exclusive-fullscreen startup fails with DXGI
diagnostic 78 (`DXGI_ERROR_INVALID_CALL`). Child windows remain unchanged.

This is host window integration; it does not change the selected display mode,
resolution, refresh rate or presentation policy.
