# Ruiss workspace instructions

- All Cargo and Tauri build, check, test, and bundle outputs for this project must use `D:/ruiss-target`.
- The project-level Cargo configuration already enforces this location. Do not create or use `src-tauri/target` on the C drive.
- Do not run `cargo clean` or delete `D:/ruiss-target` unless the user explicitly requests a full cache rebuild.
