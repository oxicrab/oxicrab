#!/bin/sh
# Stop and disable the service
if [ -d /run/systemd/system ]; then
    systemctl stop oxicrab.service || true
    systemctl disable oxicrab.service || true
    systemctl daemon-reload
fi
