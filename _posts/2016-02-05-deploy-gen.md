---
layout: post
title: "CS241 Automatic Deploy Generation"
summary: making life easier for course staff
---

I'm on the course staff for the systems programming course here at the
University of Illinois, leading the team responsible for the our large (often 2
week) assignments. When we prepare assignments we usually write up our own
solution to the assignment, then trim the solution down and provide the students
with some skeleton code. In the past, this trimming down has been done by hand.
Doing it by hand really sucks. It's very easy to miss something important, and
it becomes much more painful to make any changes to the assignment (the changes
must be made in the solution directory and the student code directory). I wanted
to alleviate this pain a bit.

What should be deployed?
------------------------
We obviously want to deploy code, but it would be nice to do a few things to the
code first. We probably want to stick some sort of header on every file, like

{% highlight c %}
/*
 * CS 241 - System Programming: Spring 2016
 * MP 0
 */
{% endhighlight %}

This is a pretty nice example of something that we don't want to change on every
single file every semester. It would probably also be nice to ensure that all
line endings are unix line endings (maybe no one other than me actually cares
about this), and to run the code through some sort of formatter (I prefer
clang-format with llvm style).

For the actual code, it might be nice if we could use c `ifdefs` to control what
is considered deploy code and what is considered solution code. In one of our
current assignments we push out intentionally broken code, so we can't just mark
code as "solution code" and remove it.

Maybe something like this would be nice:

{% highlight c %}
int main() {
#ifdef DEPLOY
    // student code goes here
    return 0;
#else
    int awesome_thing = awesome_syscall(cryptic_argument);
    return awesome_thing % 10 << 1;
#endif
}
{% endhighlight %}

This turns out to be really easy to do. There's a simple utility called
`unifdef` that just works. Run `unifdef -DDEPLOY` on the solution file with
these `ifdef` guards, and you get exactly what you wanted out:

{% highlight c %}
int main() {
    // student code goes here
    return 0;
}
{% endhighlight %}

We also probably want to copy over the Makefile for the assignment, but this
doesn't need any modification. A few of our assignments need data files or
example input files and things like that. These also don't need any
modification.

Finally, it might be nice to be able to generate a reference implementation from
our working solution code. This would need to be generated from an optimized
build of the solution code, without debug symbols (let's run it through strip
just to be safe).

Making Scripts
--------------
We need to pick some sort of tool that is really good at running command line
tools (clang-format, dos2unix, unifdef) and really good at matching patterns in
filenames. I know of at least one tool that is pretty good at both of these
things. I decided to use GNU make to define these scripts.

First, lets consider what happens to any c source file.

{% highlight make %}
$(DEPLOY_DIR)/%.c: $(DEPLOY_DIR)
	@echo "creating" $@
	@mkdir -p $(shell dirname $@)
	@unifdef -DDEPLOY $(@:$(DEPLOY_DIR)/%=$(SOURCE_DIR)/%) | $(FORMATTER) > $@-temp
	@cat $(CODE_HEADER) $@-temp | dos2unix > $@
	@rm $@-temp
{% endhighlight %}

Any `%.c` target in the deploy directory, first, depends on the existence of the
deploy directory. Then, we have to make sure the file's directory actually
exists (we might have code in `libs/weirdthing/cool_file.c`). Then, the code
runs through `unifdef` and a formatter, we stick the header on it, and pipe it
through `dos2unix`. That was easy! Well, not quite. If you aren't familiar with
GNU make, this bit might throw you off a bit:

{% highlight make %}
$(@:$(DEPLOY_DIR)/%=$(SOURCE_DIR)/%)
{% endhighlight %}

This is a patterned replacement ([Text
Function](https://www.gnu.org/software/make/manual/html_node/Text-Functions.html)
in make terminology). It basically says, take the string `$@` (the target of the
rule), and replace the text `$(DEPLOY_DIR)` with `$(SOURCE_DIR)`. In other
words, to generate a file `deploy/test.c` we would start with the file
`solution/test.c`

We do the same thing for `*.h` files.

Generating a reference implementation is also pretty straightforward:

{% highlight make %}
$(DEPLOY_DIR)/%-reference:
	@echo "making reference for" $@
	@make -s -C $(SOURCE_DIR)
	@cp $(@:$(DEPLOY_DIR)/%-reference=$(SOURCE_DIR)/%) $@
	@strip -s $@
	@make clean -s -C $(SOURCE_DIR)
{% endhighlight %}

We run make in the solution directory, then copy the executable with name
given in the target into the deploy directory, then run strip.

For anything else:

{% highlight make %}
$(DEPLOY_DIR)/%: $(DEPLOY_DIR)
	@echo "copying" $@
	@mkdir -p $(shell dirname $@)
	@-cp $(@:$(DEPLOY_DIR)/%=$(SOURCE_DIR)/%) $@
{% endhighlight %}

Just copy the file. This rule comes last, so that make will fall through to this
rule. Again, you see a bunch of text replacement going on.

You may have noticed that each of my rules is for something in the deploy
directory, then I do some text replacement to get the intended original source.
This is intentional.  There are a few cases where I do text replacement to
generate the output file name, then do text replacement again to get the input
file name back, but I'm not writing the Makefile to process input files, I'm
writing it so that it knows how to generate the appropriate output files (since
that's the Makefile model).

Here's what I mean:

{% highlight make %}
# recursively fetch all of the C and H files in the source directory
C_SOURCES := $(shell find $(SOURCE_DIR) -name '*.c')
H_SOURCES := $(shell find $(SOURCE_DIR) -name '*.h')

.PHONY: all
all: clean $(DEPLOY_DIR) code_files other_files $(REFERENCE_IMPL:%=$(DEPLOY_DIR)/%-reference)

# these targets define which files we want to build in the deploy directory
# they do not specify how to build them
.PHONY: code_files
code_files: $(C_SOURCES:$(SOURCE_DIR)/%=$(DEPLOY_DIR)/%) $(H_SOURCES:$(SOURCE_DIR)/%=$(DEPLOY_DIR)/%)

.PHONY: other_files
other_files: $(OTHER_FILES:%=$(DEPLOY_DIR)/%)
{% endhighlight %}

Using the Makefile
------------------
Our git repo looks like this:

{% highlight text %}
.
├── mp1
│   └── solution
└── mp2
    └── solution
...
{% endhighlight %}

Where each of the mp folders holds a single assignment (we call our assignments
"Machine Problems"). I stuck the Makefile described above in the root of the
repo, then define an assignment specific Makefile in the assignment directory.

{% highlight text %}
.
├── Deploy.mk
├── mp1
│   ├── Makefile
│   └── solution
└── mp2
    ├── Makefile
    └── solution
...
{% endhighlight %}

A project specific Makefile `mp1/Makefile` defines the variables the full
Makefile needs to function, and includes the full Makefile. For example:

{% highlight make %}
DEPLOY_DIR=deploy
SOURCE_DIR=solution

# Every file listed will be copied into the deploy directory with no
# modification
OTHER_FILES = Makefile \
              test_file.txt \
              a/b/c/test.dat

REFERENCE_IMPL=mp1

-include ../Deploy.mk
{% endhighlight %}

Finally, stick a file in `/mp1` called `code_header` that defines the header to
stick on each code file.

Now, running `make` in the `/mp1` directory creates the folder `deploy`
containing the deployable code, and a reference implementation for the
assignment.

Now, my life is a lot easier.


Full Deploy.mk file
-------------------
not using a gist cause I tend to accidentally delete those:
{% highlight make %}
# this is a makefile which is included by project specific makefiles
# this file defines how to build a deploy directory automatically from a
# solution directory using #ifdef DEPLOY #endif to specify deploy and solution
# specific code
#
# the contents of code_header (in the same directory as the MP Specific
# makefile) are prepended to every code file

# TODO run an extraneous header removal tool
# (http://include-what-you-use.org/)

FORMATTER=clang-format -style=llvm

# recursively fetch all of the C and H files in the source directory
C_SOURCES := $(shell find $(SOURCE_DIR) -name '*.c')
H_SOURCES := $(shell find $(SOURCE_DIR) -name '*.h')

CODE_HEADER=code_header

.PHONY: all
all: clean $(DEPLOY_DIR) code_files other_files $(REFERENCE_IMPL:%=$(DEPLOY_DIR)/%-reference)

# these targets define which files we want to build in the deploy directory
# they do not specify how to build them
.PHONY: code_files
code_files: $(C_SOURCES:$(SOURCE_DIR)/%=$(DEPLOY_DIR)/%) $(H_SOURCES:$(SOURCE_DIR)/%=$(DEPLOY_DIR)/%)

.PHONY: other_files
other_files: $(OTHER_FILES:%=$(DEPLOY_DIR)/%)

# these targets specify how to build the files that get deployed
# for example, a deploy_dir/%.c file is processed with unifdef then formatted,
# and emitted to the deploy directory
$(DEPLOY_DIR):
	@mkdir -p $(DEPLOY_DIR)

# process each of the c and header files with unifdef and a formatter
$(DEPLOY_DIR)/%.c: $(DEPLOY_DIR)
	@echo "creating" $@
	@mkdir -p $(shell dirname $@)
	@unifdef -DDEPLOY $(@:$(DEPLOY_DIR)/%=$(SOURCE_DIR)/%) | $(FORMATTER) > $@-temp
	@cat $(CODE_HEADER) $@-temp | dos2unix > $@
	@rm $@-temp

$(DEPLOY_DIR)/%.h: $(DEPLOY_DIR)
	@echo "creating" $@
	@mkdir -p $(shell dirname $@)
	@unifdef -DDEPLOY $(@:$(DEPLOY_DIR)/%=$(SOURCE_DIR)/%) | $(FORMATTER) > $@-temp
	@cat $(CODE_HEADER) $@-temp | dos2unix > $@
	@rm $@-temp

$(DEPLOY_DIR)/%-reference:
	@echo "making reference for" $@
	@make -s -C $(SOURCE_DIR)
	@cp $(@:$(DEPLOY_DIR)/%-reference=$(SOURCE_DIR)/%) $@
	@strip -s $@
	@make clean -s -C $(SOURCE_DIR)

# if we didn't catch it yet, just copy the file
$(DEPLOY_DIR)/%: $(DEPLOY_DIR)
	@echo "copying" $@
	@mkdir -p $(shell dirname $@)
	@-cp $(@:$(DEPLOY_DIR)/%=$(SOURCE_DIR)/%) $@

.PHONY: clean
clean:
	@echo "cleaning up old deploy directory"
	@rm -rf $(DEPLOY_DIR)
{% endhighlight %}
