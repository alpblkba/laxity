# shared helpers for the scripts in this directory. sourced, never executed.
#
# it sets no shell options, so whatever the caller put in its own set line stays in force. the
# scripts here do not agree on those: doctor.sh runs without -e because it reports every problem
# it finds rather than stopping at the first.
#
# what is shared is the mechanism. the policy stays with the caller, because the callers do not
# want the same thing: doctor.sh asks whether a probe is present at all, capture.sh refuses on
# anything other than exactly one.

# resolve timeout(1). macOS ships neither name and coreutils installs it as one or the other
# depending on how it was installed, so resolve it rather than assuming. prints the path and
# returns non-zero when neither exists, which leaves the message to the caller.
laxity_timeout() {
  command -v timeout 2>/dev/null || command -v gtimeout 2>/dev/null || return 1
}

# the ST-LINK listing exactly as the CLI prints it, colouring included.
#
# whatever consumes this has to read all of it. grep -q exits at the first match, the CLI then
# dies of SIGPIPE with status 141, and pipefail promotes that into a failed pipeline, which is
# how doctor.sh once reported no board with the board attached. grep -c reads to the end.
laxity_stlink_list() {
  STM32_Programmer_CLI -l 2>/dev/null
}

# every ST-LINK virtual COM port, one "<serial><tab><node>" line each, narrowed to one board when
# LAXITY_STLINK_SN is set. the node number is assigned at enumeration, so nothing may pick by
# position; the serial and the description are the only stable keys. awk reads the whole stream.
laxity_stlink_ports() {
  laxity_stlink_list \
    | sed $'s/\033\\[[0-9;]*m//g' \
    | awk -v want="${LAXITY_STLINK_SN:-}" '
        function trim(s) { sub(/^[^:]*:[[:space:]]*/, "", s); sub(/[[:space:]]+$/, "", s); return s }
        /^ST-LINK SN[[:space:]]*:/  { serial = trim($0) }
        /^Location[[:space:]]*:/    { loc = trim($0) }
        /^Description[[:space:]]*:/ {
          desc = trim($0)
          if (desc ~ /STLINK/ && loc ~ /^\/dev\/cu\./ && (want == "" || want == serial)) {
            print serial "\t" loc
          }
          loc = ""; desc = ""
        }'
}
