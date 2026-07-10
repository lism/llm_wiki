#!/bin/bash
# Server resource check via SSH
HOST="199.66.62.239"
USER="root"
PASS="admin@123"

# Create a temporary askpass script
ASKPASS=$(mktemp)
cat > "$ASKPASS" << EOF
#!/bin/bash
echo "$PASS"
EOF
chmod +x "$ASKPASS"

export SSH_ASKPASS="$ASKPASS"
export DISPLAY=dummy

# Run SSH with askpass, forcing password auth
SSH_ASKPASS="$ASKPASS" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no "$USER@$HOST" '
echo "=== MEMORY ==="
free -h
echo ""
echo "=== DISK ==="
df -h /
echo ""
echo "=== TOP 15 CPU ==="
ps aux --sort=-%cpu | head -16
echo ""
echo "=== TOP 15 MEM ==="
ps aux --sort=-%mem | head -16
echo ""
echo "=== LOAD ==="
uptime
' 2>&1

rm -f "$ASKPASS"
