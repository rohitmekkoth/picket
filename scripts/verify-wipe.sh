#!/bin/sh
# Checks that Picket's uninstall and wipe left nothing behind for this user: every
# path and Keychain item Picket creates (see crates/app/src/wipe.rs). Exits 1 and
# lists what remains otherwise. macOS privacy grants (Full Disk Access,
# notifications) and the app bundle itself are outside an app's reach and not checked.
set -u
identifier="${1:-dev.picket.desktop}"
library="$HOME/Library"
remaining=0

check_path() {
  if [ -e "$1" ]; then
    echo "remains: $1"
    remaining=1
  fi
}

if pgrep -x picket-app >/dev/null; then
  echo "Picket is still running"
  remaining=1
fi
for name in "$identifier" picket-app; do
  check_path "$library/Caches/$name"
  check_path "$library/WebKit/$name"
done
check_path "$library/Application Support/$identifier"
check_path "$library/Preferences/$identifier.plist"
check_path "$library/Saved Application State/$identifier.savedState"
check_path "$library/HTTPStorages/$identifier"
check_path "$library/LaunchAgents/Picket.plist"
if security find-generic-password -s Picket.Google >/dev/null 2>&1; then
  echo "remains: Keychain item Picket.Google"
  remaining=1
fi
if launchctl print "gui/$(id -u)/Picket" >/dev/null 2>&1; then
  echo "remains: loaded login item Picket"
  remaining=1
fi

if [ "$remaining" -eq 0 ]; then
  echo "PASS: no Picket data, settings, Keychain items or login item remain"
fi
exit "$remaining"
