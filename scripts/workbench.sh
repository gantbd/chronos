#!/bin/sh
# Source-checkout launcher only. No installation, background process, retries,
# implicit writes or authority elevation. MCP stdout remains protocol-only.
set -eu
chronos_workbench_root=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)
chronos_workbench_parent=$(dirname "$chronos_workbench_root")
chronos_workbench_usage() {
    cat <<'USAGE'
Usage: sh scripts/workbench.sh COMMAND [ABSOLUTE_SESSION_DIRECTORY] [OPTIONS]
  build-core             Build the service and MCP connector offline
  build-tui | build-gui   Build one independent presentation offline
  service DIR [OPTIONS]  Run the service in the foreground
  agent DIR [OPTIONS]    Run the MCP stdio connector for a harness
  tui DIR [OPTIONS]      Run the terminal client
  gui DIR [OPTIONS]      Run the macOS GUI client
  status DIR             Read service readiness
  shutdown DIR           Request graceful service drain
  help                   Show this help
Task writes require explicit --allow-writes at the service AND client.
Agent/UI clients connect to the same running service and directory.
No execution harness, live provider, remote authentication or installer is supplied.
USAGE
}
chronos_workbench_exec() {
    chronos_workbench_binary=$1
    shift
    if [ ! -x "$chronos_workbench_binary" ]; then
        echo "Required binary is missing; run the corresponding build command first." >&2
        exit 2
    fi
    exec "$chronos_workbench_binary" "$@"
}
chronos_workbench_command=${1:-help}
if [ "$#" -gt 0 ]; then shift; fi
case "$chronos_workbench_command" in
    help|--help)
        chronos_workbench_usage
        exit 0
        ;;
    build-core|build-tui|build-gui)
        if [ "$#" -ne 0 ]; then echo "Build commands take no arguments." >&2; exit 2; fi
        case "$chronos_workbench_command" in
            build-core)
                exec cargo build --manifest-path "$chronos_workbench_root/Cargo.toml" --target-dir "$chronos_workbench_root/target" --locked --offline -p chronos-standalone -p chronos-mcp --bins
                ;;
            build-tui) chronos_workbench_presentation=chronos-tui ;;
            build-gui) chronos_workbench_presentation=chronos-gui-macos ;;
        esac
        exec cargo build --manifest-path "$chronos_workbench_parent/$chronos_workbench_presentation/Cargo.toml" --target-dir "$chronos_workbench_parent/$chronos_workbench_presentation/target" --bin "$chronos_workbench_presentation" --locked --offline
        ;;
    service|agent|tui|gui|status|shutdown)
        if [ "$#" -eq 0 ]; then echo "Absolute session directory required." >&2; exit 2; fi
        case "$1" in /*) ;; *) echo "Session directory must be absolute so every client resolves the same resource." >&2; exit 2 ;; esac
        chronos_workbench_directory=$1
        shift
        case "$chronos_workbench_command" in
            service) chronos_workbench_exec "$chronos_workbench_root/target/debug/chronos-service" "$chronos_workbench_directory" "$@" ;;
            agent) chronos_workbench_exec "$chronos_workbench_root/target/debug/chronos-mcp" "$chronos_workbench_directory" "$@" ;;
            tui) chronos_workbench_exec "$chronos_workbench_parent/chronos-tui/target/debug/chronos-tui" "$chronos_workbench_directory" "$@" ;;
            gui) chronos_workbench_exec "$chronos_workbench_parent/chronos-gui-macos/target/debug/chronos-gui-macos" "$chronos_workbench_directory" "$@" ;;
            status|shutdown)
                if [ "$#" -ne 0 ]; then echo "Control commands take only the directory." >&2; exit 2; fi
                chronos_workbench_exec "$chronos_workbench_root/target/debug/chronos-service" "--$chronos_workbench_command" "$chronos_workbench_directory"
                ;;
        esac
        ;;
    *) echo "Unknown command. Use help." >&2; exit 2 ;;
esac
