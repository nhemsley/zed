#!/bin/bash

# Script to clear the MRU (Most Recently Used) models table

DB_PATH="$HOME/.local/share/zed/threads/threads.db"

if [ ! -f "$DB_PATH" ]; then
    echo "Error: Database not found at $DB_PATH"
    exit 1
fi

echo "Clearing MRU table from: $DB_PATH"

# Check if Zed is running
if pgrep -x "zed" > /dev/null; then
    echo "Warning: Zed is currently running. Please close Zed first to avoid database locks."
    read -p "Press Enter to continue anyway or Ctrl+C to cancel..."
fi

# Clear the MRU table
sqlite3 "$DB_PATH" "DELETE FROM model_mru;"

if [ $? -eq 0 ]; then
    echo "✓ MRU table cleared successfully!"

    # Show the count
    COUNT=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM model_mru;")
    echo "  Remaining entries: $COUNT"
else
    echo "✗ Failed to clear MRU table"
    exit 1
fi
