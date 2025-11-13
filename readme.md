# Telera Application Framework

a GUI app framework designed for performance, and modularity.

## Features:
- build UI elements in a custom script or directly in code
- UI elements are modular and reusable
- UI is immediate mode and definitionaly reactive.
    - Only the layout is stored in script, all the data lives in *your* user application
- UI layout is similar to CSS's flex layout system.
- Hardware rendering of the UI
    - rendering can automatically be switched to CPU if no graphics card is found
    - small layouts can render on the order of microseconds
- Included convenience macros remove most of the Rust boilerplate
    - go from empty folder to running application in minutes!
    - complexity can scale with requirements


## Roadmap:
- v0.5.0: In progress
    - build ui with custom script
        - recursive reusable components
        - access application data with macros simply by matching the variable name in your script
        - hot reloading of UI
        - bundle UI script in application binary or specify user directory for text files
    - build application code with Safe and Performant Rust
        - cross platform:
            - Windows
            - Arch Linux
        - multi window support
        - able to compile and run with < 100 lines of Rust
        - startup immediately from template project
    - 3D rendering capabilities built in
        - convenient api to load/manipulate GLTF models
    - Known Bugs:
        - srcipt data access not working reliably
        - Text input not possible
        - UI components not working/recursive
        - user can't set ui script directory
        - no tests
        - multi-window page setting from user app not working
        - independent border widths not possible
- v0.6.0 ~ v0.9.0: feature set not decided
- v1.0
    - rich set of highly customizable UI widgets
    - reactive animations for any/all UI configuration settings
    - fully cross-platform (no native mobile)
    - batch rendering of UI for performance
    - expose api for user created UI/3D Shaders
    - expose api for user Graphics middleware