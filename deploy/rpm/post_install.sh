#!/bin/sh
# Create system user if it doesn't exist
if ! getent passwd oxicrab >/dev/null 2>&1; then
    useradd --system --user-group --home-dir /var/lib/oxicrab \
        --no-create-home --shell /sbin/nologin \
        --comment "oxicrab AI assistant" oxicrab
fi

# Create directories with correct ownership
install -d -m 0750 -o oxicrab -g oxicrab /var/lib/oxicrab
install -d -m 0750 -o oxicrab -g oxicrab /var/lib/oxicrab/workspace
install -d -m 0750 -o oxicrab -g oxicrab /etc/oxicrab

# Install example config if no config exists
if [ ! -f /etc/oxicrab/config.json ]; then
    if [ -f /usr/share/doc/oxicrab/config.example.json ]; then
        install -m 0640 -o oxicrab -g oxicrab \
            /usr/share/doc/oxicrab/config.example.json \
            /etc/oxicrab/config.json
        echo "oxicrab: installed example config to /etc/oxicrab/config.json"
        echo "oxicrab: edit it with your API keys, then run: systemctl start oxicrab"
    fi
fi

# Reload systemd and enable (but don't start) the service
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload
    systemctl enable oxicrab.service || true
    echo "oxicrab: service enabled. Start with: systemctl start oxicrab"
fi
