A simple work/break interval timer: start, pause, adjustable work and break
lengths, notifications when it's time for a break, a menu bar icon, and a count
of the hours you work each day.

## Install

### macOS (Apple Silicon and Intel, macOS 11 or later)

1. Download **TimerShift-macos-universal.zip** and double-click it to unzip.
2. Drag **TimerShift.app** into your **Applications** folder.
3. The app isn't notarized by Apple, so the first time you open it macOS says it
   can't verify it and won't open it. Either:
   - after that first attempt, open **System Settings › Privacy & Security**,
     scroll down and click **Open Anyway** next to TimerShift, or
   - run this once in Terminal:
     `xattr -dr com.apple.quarantine /Applications/TimerShift.app`
4. Click **Allow** when macOS asks whether TimerShift may send notifications.

### Windows (64-bit)

Download **TimerShift-windows-x86_64.zip**, unzip it and run **TimerShift.exe**.
If SmartScreen warns about an unrecognised app, click **More info › Run anyway**.

### Linux (x86_64)

Download **TimerShift-linux-x86_64.tar.gz**, then:

```bash
tar -xzf TimerShift-linux-x86_64.tar.gz
./timershift
```

The menu bar (tray) icon isn't available on Linux yet.
