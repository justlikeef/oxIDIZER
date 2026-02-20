#!/bin/bash
# scripts/sync_and_link_configs.sh

CONF_ACTIVE_DIR="/var/repos/oxIDIZER/conf/modules/active"
WORKSPACE_DIR="/var/repos/oxIDIZER"

set -e

for config_file in "$CONF_ACTIVE_DIR"/*.yaml; do
    [ -e "$config_file" ] || continue
    if [ -L "$config_file" ]; then
        echo "Skipping already linked: $config_file"
        continue
    fi
    
    filename=$(basename "$config_file")
    modname="${filename%.yaml}"
    
    # Try to find the module source directory
    # We look in the root and in subdirectories like ox_persistence/
    mod_src_dir=$(find "$WORKSPACE_DIR" -maxdepth 3 -type d -name "$modname" | head -n 1)
    
    if [ -n "$mod_src_dir" ] && [ -d "$mod_src_dir/conf" ]; then
        target_src="$mod_src_dir/conf/$filename"
        if [ -e "$target_src" ]; then
            # Compare timestamps
            if [ "$config_file" -nt "$target_src" ]; then
                echo "Syncing newer config: $config_file -> $target_src"
                cp -p "$config_file" "$target_src"
            fi
            echo "Soft-linking $config_file -> $target_src"
            mv "$config_file" "$config_file.bak"
            ln -s "$target_src" "$config_file"
            rm "$config_file.bak"
        else
            echo "Warning: Target source $target_src does not exist. Skipping link."
        fi
    else
        echo "Warning: No module conf dir found for $modname. Skipping link."
    fi
done
