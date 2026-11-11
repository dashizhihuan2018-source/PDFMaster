-- Enable native file previews in a Finder folder, without
-- activating Finder, opening documents, changing defaults, or restarting it.
-- Usage: osascript scripts/enable-finder-previews.applescript /absolute/folder
on run argv
    if (count of argv) is not 1 then error "Pass one absolute folder path." number 64
    set requestedPath to item 1 of argv
    if requestedPath does not start with "/" then error "Folder path must be absolute." number 64
    set requestedFolder to (POSIX file requestedPath) as alias
    tell application "Finder"
        set folderWindow to container window of folder requestedFolder
        tell list view options of folderWindow
            set shows icon preview to true
            set icon size to large icon
        end tell
        set shows icon preview of icon view options of folderWindow to true
        set shows icon preview of column view options of folderWindow to true
        if (shows icon preview of list view options of folderWindow) is not true then error "Finder did not enable previews."
        if (icon size of list view options of folderWindow) is not large icon then error "Finder did not retain the larger icons."
    end tell
    return "Enabled native previews and large list icons for the requested folder."
end run
