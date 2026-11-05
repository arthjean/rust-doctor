set -u
tool=$(basename "$0")
log=${RD_FAKE_LOG:?RD_FAKE_LOG is not set}
mkdir -p "$log"
n=1
while [ -e "$log/$tool.$n.argv" ] || [ -e "$log/$tool.$n.cwd" ]; do n=$((n + 1)); done
base="$log/$tool.$n"
pwd > "$base.cwd"
if [ -t 0 ]; then
  : > "$base.stdin"
  echo tty > "$base.tty"
else
  cat > "$base.stdin"
fi
for arg in "$@"; do printf '%s\0' "$arg"; done > "$base.argv.partial"
mv "$base.argv.partial" "$base.argv"
name=$(printf '%s' "$tool" | tr 'a-z-' 'A-Z_')
eval "pause=\${RD_FAKE_${name}_SLEEP:-0}"
eval "code=\${RD_FAKE_${name}_EXIT:-0}"
[ "$pause" = 0 ] || sleep "$pause"
exit "$code"
