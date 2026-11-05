#!/bin/sh
# becomes /usr/trimui/bin/runtrimui.sh on the Brick, as NextUI's boot wrapper:
# boot into .tmp_update/updater when the card has one, stock otherwise
mounted=`cat /proc/mounts | grep -i SDCARD`
cnt=0
while [ "$mounted" == "" ] && [ $cnt -lt 6 ] ; do
   sleep 0.5
   cnt=`expr $cnt + 1`
   mounted=`cat /proc/mounts | grep -i SDCARD`
done

UPDATER_PATH=/mnt/SDCARD/.tmp_update/updater
if [ -f "$UPDATER_PATH" ]; then
	"$UPDATER_PATH"
else
	/usr/trimui/bin/runtrimui-original.sh
fi
