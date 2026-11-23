# Security policy

## Supported versions

Only the latest release of Corolla receives fixes. It supports Windows 10 and 11 (64-bit) and
macOS 13 or newer on Apple silicon.

## What counts as a security problem

Corolla opens whatever files are dropped on it and hands them to conversion programs, so
anything that lets a file do more than be converted is a security problem. For example: a file
or archive that makes Corolla write outside the folder it came from, overwrite an existing
file, run a command, or read files it wasn't given; or a file name that a conversion program
reads as an option.

Corolla doesn't use the network. The conversion programs it carries (FFmpeg, ImageMagick,
7-Zip, PDFium, Assimp, Mayo, FontForge) are updated with each release; a problem inside one of
them is best reported to that project as well.

## Reporting a problem

**Please don't open a public issue.** Report it privately instead: go to the repository's
**Security** tab and choose **Report a vulnerability**
([direct link](https://github.com/mttfntn/corolla-tool/security/advisories/new)).

Please describe the problem and how to reproduce it, with a sample file if you can. You'll get
a reply as soon as possible, and the fix will be released before the details are made public.
