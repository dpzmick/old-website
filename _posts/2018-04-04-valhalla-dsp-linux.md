---
layout: post
title: "Windows VST (ValhallaDSP Freq Echo) on Bitwig, on Arch Linux"
summary: "Mostly notes for myself when I invevitably have to figure this out again"
category:
tags: []
---

As a test of my ability to coherce open source music software into actually working on my machine, I attempted to get the free [ValhallaDSP Freq Echo VST plugin](https://valhalladsp.com/shop/delay/valhalla-freq-echo/) to run inside of [Bitwig Studio](https://www.bitwig.com/en/home.html).
This plugin is only distributed as a windows installer.
The installer will place some `.dll` files in magic spots that windows DAWs could look to run the plugin.

I'm running linux, so none of this really works very well for me, but I'm determined to make linux works as my music production environment (as a sidenote, bitwig studio is amazing and it runs flawlessly on linux).
Fortunately, [others](https://breakfastquay.com/dssi-vst/) [are](https://github.com/kmatheussen/ladspavst) [also](https://github.com/osxmidi/LinVst) [determined](https://github.com/phantom-code/airwave).

Airwave
=======
[Airwave](https://github.com/phantom-code/airwave) is a totally insane solution for running windows VST plugins in linux audio software.
It essentially runs the windows VST in [wine](https://www.winehq.org/), then creates a fake mini linux VST that shuffles signals to and from the windows version running in wine.
I've never had tons of luck with wine, but the internet said this actually works pretty well, and that airwave was the way to go.

Before installing airwave, you'll need to get [multilib packages](https://wiki.archlinux.org/index.php/official_repositories#Enabling_multilib) for arch setup.
Multilib just lets your package manager install 32 bit version of 64 bit packages along side the 64 bit versions.
Wine needs this for some reason.
Bitwig apparently can't run the 64 bit VSTs either, so you'll need mutlilib to run the 32 bit airwave wrappers (this might be false, I didn't try very hard).

Install airwave from the AUR (I use [cower](https://aur.archlinux.org/packages/cower) as an AUR wrapper):

Install Dependencies:
```sh
$ sudo pacman -S wine
$ cower -d steinberg-vst36
:: steinberg-vst32 downloaded to /home/dzmick/builds
$ cd # wherever the package went
$ makepkg -si
```

Install package:
```sh
$ cower -d airwave-git
:: airwave-git download to /home/dpzmick/builds
$ cd airwave-git
$ makepkg -si
```

Install VST
===========

I downloaded the windows valhalla DSP plugin here: <https://valhalladsp.com/shop/delay/valhalla-freq-echo/>.
Then:

```sh
$ unzip ValhallaFreqEchoWin_V1_0_5.zip # exe ends up in cwd
$ wine ValhallaFreqEchoWin_V1_0_5.exe
```

Wine might complain about some packages it thinks are missing.
I didn't need either of them.

Click through the windows installer.
Install both of the VST plugins (RTAS and AAX aren't needed).
The installer will ask for a windows path; this is a path in your wine "C" drive (probably `~/.wine/drive_c/` or something).
Next, Next, Finish.

You'll also need to install this package on arch: `multilib/lib32-mpg123`

```
$ sudo pacman -S lib32-mpg123
```

None of the depencencies picked this up for some reason; airwave can't figure out what's happening when it can't load the plugins.
If you aren't on arch and are having issues, you might be missing the 32 bit version of libmpg123.

Run Airwave
===========
Run `airwave-manager` and follow the instructions from the airwave readme (reproduced here sort of).

1. Click create link button.
2. Leave "WINE loader" and "WINE prefix" default, unless you changed them.
3. For "VST Plugin" path, use the path you gave the VST installer above. This is the `dll` airwave will wrap. Select the 32 bit version.
4. For "Link Location" provide a path to a directory where airwave should place the `so` file it will generate. Linux DAWs will need to search this path to pickup the plugin.
5. Leave "Link name" default. I think it might be a good idea to keep this is sync with the actual plugin names. Opening some DAW project on windows might actually work if the plugin names are in sync.
6. Click "OK"

I haven't had much luck editing links, it seems more reliable to delete it and start over if something isn't working properly.

Start up Bitwig and the plugin should be addable, the graphics will probably even work!
![bitwig with windows VST running](/img/bitwig/bitwig.png)

It's pretty cool that this actually works.
