---
layout: post
title: "Vim Remote Make"
description: ""
category:
tags: []
---

Remote Make
-----------
So I'm in a class that requires that I use a linux VM to edit code. Since we are
sharing this VM between all of the group members, I don't want to jump on there
and throw my vim config and stuff on the machine. Instead, I've been mounting
the remote filesystem with sshfs and editing files remotely, but then I have to
run make outside of vim to get any sort of compiler errors. I realized that I
could probably run make remotely with a script and use makeprg to run that
script, then get errors in vim (maybe even get syntastic to call the remote make
or something?), and that takes a little bit of tweaking but it works nicely.

Here's how I did it.

~/dotfiles/bin/423make:
{% highlight bash %}

#!/bin/bash
oldpath=/home/cs423/$1
newpath=/Users/dz0004455/programming/cs423/remote/$1
ssh cs423@$REMOTE_HOST "cd ~/$1 && make"  2>&1 | sed -e "s|$oldpath|$newpath|g"

{% endhighlight %}

This script runs the make program remotely (makes lots of assumptions about
directory structure, but that's totally fine). Then it pipes the output through
sed to replace all the paths make spits out on the remote machine with the paths
on the local machine, so that vim can open the files with errors correctly. If
you don't replace the pathnames, vim will try to open the wrong files and fail
to jump to the errors you wanted it to jump to.

Then, from vim, open a file you need to edit, run :set makeprg=423make\ MP2 (if
I wanted to build something in /home/cs423/MP2), and then run :make to build the
program remotely. The make program will populate the quickfix window, which you
can open with :copen.

Next step might be trying to get syntastic to run my remote make, but I'm
anxious about the inevitable slow make every time I save the file. Since I'm
using neovim I can also use the wonderful
[ neomake ]( https://github.com/benekastah/neomake ) plugin to run builds
asynchronously and populate quickfix.

avoiding sshfs
--------------
After using this for a few hours on a less than stellar internet connection, I
got a little tired of sshfs dropping my connections, so I added two more "scripts"
to this mix.

423get
{% highlight bash %}
rsync -a cs423@$REMOTE_HOST:/home/cs423/$1/ ~/programming/cs423/sync/$1/
{% endhighlight %}

423push
{% highlight bash %}
rsync -a ~/programming/cs423/sync/$1/ cs423@$REMOTE_HOST:/home/cs423/$1/
{% endhighlight %}

Then, in vim:

{% highlight vim %}
:autocmd BufWrite * :Dispatch! 423push MP2
{% endhighlight %}

so files get pushed back (via rsync) anytime I write. Again, could have used
neomake for the asynchronous behavior (instead of dispatch), but the dispatch
solution is slightly easier to type.

