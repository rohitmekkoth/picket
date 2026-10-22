-- Run against a newly launched Picket app. Requires existing Accessibility access.
-- This verifies native IPC, close-to-tray, single-instance restore, and quit.
on run argv
    if (count of argv) is not 1 then error "Pass the absolute path to the Picket executable."
    set executablePath to item 1 of argv

tell application "System Events"
    if not UI elements enabled then error "Enable Accessibility for your terminal before running native UI checks."
    -- Wait for the packaged process and its webview-backed window.
    repeat 100 times
        if exists process "picket-app" then exit repeat
        delay 0.1
    end repeat
    if not (exists process "picket-app") then error "Picket did not launch."
    tell process "picket-app"
        repeat 100 times
            if exists window "Picket" then exit repeat
            delay 0.1
        end repeat
        if not (exists window "Picket") then error "The Picket window did not appear."

        repeat 100 times
            set allElements to entire contents of window "Picket"
            set labels to {}
            repeat with e in allElements
                if role of e is "AXStaticText" then set end of labels to value of e
            end repeat
            if labels contains "Desktop core connected" then exit repeat
            delay 0.1
        end repeat
        if labels does not contain "Desktop core connected" then error "Native Rust connection is not displayed."
        if labels contains "Open the desktop app to connect Google." then error "Google connection commands are unavailable in the packaged app."
        if labels does not contain "Google Desktop OAuth client ID" and labels does not contain "Connection: monitoring" and labels does not contain "Connection: starting" then error "Google connection controls or status are missing."

        -- Closing must preserve the process and its tray icon.
        click (first button of window "Picket" whose subrole is "AXCloseButton")
        repeat 50 times
            if not (exists window "Picket") then exit repeat
            delay 0.1
        end repeat
        if exists window "Picket" then error "Closing the window did not hide it."

        if not (exists menu bar item 1 of menu bar 2) then error "The tray icon is missing."
        -- A second launch must restore the existing single instance.
        do shell script quoted form of executablePath & " >/dev/null 2>&1 &"
        repeat 50 times
            if exists window "Picket" then exit repeat
            delay 0.1
        end repeat
        if not (exists window "Picket") then error "A second launch did not restore the window."

        keystroke "q" using command down
    end tell
    -- Confirm Command-Q ends the process instead of leaving a tray orphan.
    repeat 50 times
        if not (exists process "picket-app") then exit repeat
        delay 0.1
    end repeat
    if exists process "picket-app" then error "Picket did not quit."
end tell
return "PASS: native IPC status, Google connection UI, close-to-tray, tray presence, single-instance restore, and quit"
end run
