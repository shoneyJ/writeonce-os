#!/usr/bin/env bash
# survey-target-machine.sh — collect hardware/firmware/kernel facts about the
# target ThinkPad into a single markdown file for WriteOnce OS planning.
#
# Usage:
#   sudo ./scripts/survey-target-machine.sh                 # writes ./target-machine.md
#   sudo ./scripts/survey-target-machine.sh /path/out.md    # custom output path
#
# Run with sudo so dmidecode / parted work. Non-sudo runs still produce a
# usable file — sections needing root are marked "(requires sudo)".

set -u

OUT="${1:-./target-machine.md}"
OUT="$(realpath -m "$OUT")"

# --- helpers ----------------------------------------------------------------

have()    { command -v "$1" >/dev/null 2>&1; }
hdr()     { printf '\n## %s\n\n```\n' "$1"; }
end()     { printf '```\n'; }
run()     { "$@" 2>&1 || echo "(command failed: $*)"; }
sudo_run(){ if [[ $EUID -eq 0 ]]; then "$@" 2>&1; else echo "(requires sudo — re-run with sudo to capture this section)"; fi; }
need()    { have "$1" || { echo "(missing: $1 — install with 'apt install $2')"; return 1; }; }

# --- preflight --------------------------------------------------------------

if [[ $EUID -ne 0 ]]; then
  echo "warning: not running as root — DMI, parted, and full lspci output will be partial" >&2
fi

mkdir -p "$(dirname "$OUT")"

# --- collect ----------------------------------------------------------------

{
  echo "# Target Machine Hardware Survey"
  echo
  echo "_Generated: $(date -Is)_  "
  echo "_Host: $(hostname)_  "
  echo "_Script: scripts/survey-target-machine.sh_"

  hdr "uname -a";                       run uname -a; end
  hdr "/etc/os-release";                run cat /etc/os-release; end

  hdr "DMI — system / baseboard / BIOS"
  need dmidecode dmidecode && sudo_run dmidecode -t system -t baseboard -t bios
  end

  hdr "DMI — chassis"
  have dmidecode && sudo_run dmidecode -t chassis || echo "(dmidecode missing)"
  end

  hdr "Firmware mode (UEFI / BIOS)"
  if [[ -d /sys/firmware/efi ]]; then
    echo "UEFI ($(cat /sys/firmware/efi/fw_platform_size 2>/dev/null || echo "?")-bit)"
    echo "Secure Boot: $( [[ -r /sys/firmware/efi/efivars ]] && (mokutil --sb-state 2>/dev/null || echo "(install mokutil to check)") || echo "(efivars not readable)" )"
  else
    echo "Legacy BIOS"
  fi
  end

  hdr "CPU (lscpu)";                    run lscpu; end
  hdr "CPU flags (deduped)"
  grep -m1 '^flags' /proc/cpuinfo | cut -d: -f2 | tr ' ' '\n' | sort -u | grep -v '^$' | paste -sd' '
  end

  hdr "Memory — free -h";               run free -h; end
  hdr "Memory — DMI"
  if have dmidecode && [[ $EUID -eq 0 ]]; then
    dmidecode -t memory 2>&1 | grep -E 'Size:|Speed:|Type:|Manufacturer:|Locator:|Form Factor:|Configured Memory Speed:'
  else
    echo "(requires sudo + dmidecode)"
  fi
  end

  hdr "PCI devices (lspci -nnk)"
  if have lspci; then
    sudo_run lspci -nnk
  else
    echo "(install pciutils)"
  fi
  end

  hdr "USB devices (lsusb)"
  have lsusb && run lsusb || echo "(install usbutils)"
  end

  hdr "Block devices (lsblk)"
  run lsblk -o NAME,FSTYPE,SIZE,TYPE,MOUNTPOINT,UUID,MODEL,SERIAL,ROTA
  end

  hdr "Partition tables (parted -l)"
  have parted && sudo_run parted -l || echo "(install parted)"
  end

  hdr "Storage controllers (filtered lspci)"
  have lspci && lspci -nnk 2>/dev/null | grep -A3 -iE 'sata|nvme|raid|storage|ahci|mmc|sd host' || echo "(no matches)"
  end

  hdr "Network controllers (filtered lspci)"
  have lspci && lspci -nnk 2>/dev/null | grep -A3 -iE 'ethernet|network|wireless|bluetooth' || echo "(no matches)"
  end

  hdr "Network links"
  run ip -br link
  echo
  run ip -br addr
  end

  hdr "GPU / display (filtered lspci)"
  have lspci && lspci -nnk 2>/dev/null | grep -A3 -iE 'vga|display|3d' || echo "(no matches)"
  end

  hdr "Audio (filtered lspci)"
  have lspci && lspci -nnk 2>/dev/null | grep -A3 -iE 'audio|hda' || echo "(no matches)"
  end

  hdr "Loaded kernel modules (lsmod)";  run lsmod; end
  hdr "Kernel cmdline";                 run cat /proc/cmdline; end

  hdr "Running kernel config"
  if [[ -r /proc/config.gz ]]; then
    zcat /proc/config.gz
  elif [[ -r "/boot/config-$(uname -r)" ]]; then
    cat "/boot/config-$(uname -r)"
  else
    echo "(no kernel config found at /proc/config.gz or /boot/config-\$(uname -r))"
  fi
  end

  hdr "Sensors / thermal (quick)"
  have sensors && sensors 2>&1 || echo "(install lm-sensors and run 'sudo sensors-detect' for thermal data)"
  end

  echo
  echo "---"
  echo "_Survey complete. Capture follow-up notes about target-specific decisions directly under each section._"
} > "$OUT"

echo "wrote: $OUT"
echo "size:  $(wc -l < "$OUT") lines, $(du -h "$OUT" | cut -f1)"
