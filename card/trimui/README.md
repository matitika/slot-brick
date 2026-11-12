# slot on the TrimUI Brick

Two ways to run slot on a TrimUI Brick (TG3040):

- **Stock firmware**: slot starts from TrimUI's own system. Copy files onto the card.
  Boots in about 9 seconds, as stock does.
- **Slot OS** (experimental): the Brick boots straight from the card into slot, in about
  4 seconds. Flash an image onto a card. Nothing on the Brick itself is changed: take the
  card out and it boots stock as before.

Slot OS is adapted from LordNordyx's [Slot-Brick](https://github.com/LordNordyx/Slot-Brick)
(GPL-3.0); the stock-firmware port started from davidtpb's BrickSlot and Jaizu's port of it.

## Building

The easiest way: push to a `brick` branch on GitHub (or run the `brick` workflow by hand from
the Actions tab). The run's page then has `slot-brick-stock` (the stock-firmware card, zipped)
and `slot-os-brick` (the Slot OS image) to download.

To build locally instead, both need the device build (`task dist:device`'s prerequisites: Docker on a non-arm64 machine).

Stock firmware card files, in `dist-trimui/`:

    task dist:trimui

Slot OS image, in `dist-os/slot-os.img.xz` (needs Docker; downloads TrimUI's firmware 1.1.1
once, about 300 MB, checked against `card/trimui/os/sources.txt`):

    task os:trimui

## Installing on stock firmware

1. Format the card FAT32 (or exFAT).
2. Copy everything in `dist-trimui/` to the card's root, including the hidden `.tmp_update`
   folder.
3. Put the card in the Brick and start it. The first start runs `trimui/app/MainUI`, which
   installs a small boot hook (as NextUI's installer does) and starts slot. Later starts go
   straight to slot.

## Installing Slot OS

1. Flash `dist-os/slot-os.img.xz` onto a card with Raspberry Pi Imager, balenaEtcher or
   `dd`. This erases the card.
2. Put the card in the Brick and start it. The first start grows the card's partition to
   the whole card, then slot comes up.
3. Put your games on the card (in a computer, the card's partition is the FAT one named
   SLOT).

## Boot logo (Slot OS)

The logo shown while the Brick starts is `bootlogo.bmp` at the card's root. To change it, put
a 24- or 32-bit `.bmp` at `Config/bootlogo.bmp`, from Boot Logo Studio (any device size) or
Gamebit's Boot Logo page: slot fits it to the Brick's 1024×768 screen and installs it, so it
shows from the start after next. Delete it (Gamebit: Restore) to get the Slot OS logo back.
On stock firmware the Brick keeps TrimUI's own logo.

## When something goes wrong

Logs are on the card in `Logs/`:

- `slot.log` (and `slot.previous.log`): slot's own output.
- `os.log`: Slot OS only: the kernel log, the GPU and Wi-Fi drivers, the mixer.
- `boot-times.log`: Slot OS only: when each boot step finished.
- `sleep.log`: each suspend and resume.
- `slot-session.log`: stock firmware only: each start of slot.

Set `SLOT_TRACE_INPUT=1` (stock: in `.tmp_update/updater`) to log every button event to
`Logs/input-trace.log`.

On Slot OS, a script at `slot-os/debug.sh` on the card runs before slot, its output in
`Logs/debug.log`. To try a new slot without reflashing, copy `System/slot` over the card's.
