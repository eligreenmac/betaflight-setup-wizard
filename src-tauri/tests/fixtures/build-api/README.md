# Build API option lists

`options-<release>.json`: replies of `GET https://build.betaflight.com/api/options/<release>`
for 4.4.3, 4.5.2 and 2025.12.2, fetched on 2026-10-23 and re-indented (content unchanged).
They are lists of Betaflight build-option names, defines and MSP_BUILD_INFO ids (the defines come
from the GPL-3.0 Betaflight firmware). Used by `src/flasher/options.rs` and `service.rs` tests.
