# Live stock SolidCraft checks

Build the host and components first (../../scripts/build-plugins.sh).
Start your unmodified stock app:

    solidcraft --control 7878

Run:

    SOLIDCRAFT_CONTROL_PORT=7878 python3 tests/live/run.py

Without SOLIDCRAFT_CONTROL_PORT all three tests skip. The script never starts,
stops, or modifies the application binary. Run against a disposable active design:
it imports examples/generated_unit_cube.f3d through importer-fusion, opens the cube and creates a box. Token discovery uses Bridge's
platform location, or SOLIDCRAFT_CONTROL_DIR. No private fixtures are consulted.
DUTCHMAN_PLUGINS_BIN can select a host binary; DUTCHMAN_PLUGINS_DATA is isolated
in a temporary directory. The imported design output also goes in that directory.
