# Kernel build history

> Append-only log of every kernel rebuild. Each `just kernel` run
> automatically appends an entry (see `build/kernel-history-append.sh`).
>
> Most recent entries last. Format per entry:
>
> ```
> ## <ISO-8601 timestamp UTC>
> bzImage    : <size> bytes  sha256=<hex>
> initramfs  : <size> bytes  sha256=<hex>
> git        : <short SHA> <oneline subject>
> fragment   : sha256=<hex of kernel-config-additions.fragment>
> reason     : <free-form one-liner explaining why this build happened>
> ```

---

## 2026-05-25T20:04:52Z
bzImage    :     15086592 bytes  sha256=b476376496046d30776aeedbc07672c267575a187044f911fa90384ce34ad04a
initramfs  :      2412935 bytes  sha256=791f8a4e08bbe7eb139b59cc93a97aedb5ed6d77b4b630dcddb69aab0a26acde
git        : 9e84aaa  LSF build packages
fragment   : sha256=7c9f64553fc1…
reason     : Add CONFIG_DRM_SIMPLEDRM + CONFIG_SYSFB_SIMPLEFB — universal-fallback framebuffer for T450 i915-transition blackout

## 2026-05-25T21:54:00Z
bzImage    :     15622656 bytes  sha256=3bec7b4d227e263c1aae25db7e6c0de1c2f43218e4ef1ea3d1f265c5d3b2ad7d
initramfs  :       782768 bytes  sha256=b5a68a17612cfde9038cd8d597a826ec56acb955b68d3bb30627d78dcfb7d3c2
git        : 9e84aaa  LSF build packages
fragment   : sha256=7c9f64553fc1…
reason     : switch base config: defconfig → Ubuntu 6.8 LTS (stripped certs)

## 2026-05-26T01:21:57Z
bzImage    :     15651328 bytes  sha256=6433012457ab8506abc7613f18a52ba80112c8a6d868b8472ebacdc2ce7b7ea8
initramfs  :       797382 bytes  sha256=e5cf780811bf1173a4d7d27ae31b420bd511bcdab14ee8b637a858e53b3ccdd8
git        : 9e84aaa  LSF build packages
fragment   : sha256=0b2b71470eaf…
reason     : Pin USB_STORAGE + USB_UAS as built-in; add rootwait to cmdline. Boot-from-USB chicken-and-egg fix.

## 2026-05-27T05:58:02Z
bzImage    :     16531968 bytes  sha256=e50f3f475ba22ad01d01cbe602e687b00aa49d3e2fd09e99351350d170314b41
initramfs  :      1796375 bytes  sha256=1788826f2aa87860ae8bc54795ee2aaf46e17233d50a83825fe046e68154b1af
git        : 9e84aaa  LSF build packages
fragment   : sha256=40872217b9e7…
reason     : reason=add iwd CRYPTO_USER_API + cfg80211/mac80211 builtin; pipewire SND_HRTIMER
