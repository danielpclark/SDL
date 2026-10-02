# Licensing of a Rust translation of SDL

*This document explains the reasoning behind the license choice for this
repository. It is not legal advice; for a product decision, talk to a lawyer.*

## The short answer

**A translation of SDL is a derivative work of SDL, so SDL's license terms
follow it.** Rewriting C into Rust by hand or by machine does not create an
independent, un-encumbered work: copyright protects the expression of the
program (its structure, sequence, organisation, comments, tables, algorithmic
choices), not merely the literal characters, and a faithful translation
carries all of that over.

The good news is that SDL is under the **zlib license**, one of the most
permissive licenses in existence. It says, in full effect:

1. you may use the software for any purpose, including commercially;
2. you may alter it and redistribute it freely;
3. you must not misrepresent its origin or claim you wrote the original;
4. altered versions must be plainly marked as such;
5. the notice may not be removed from any source distribution.

Nothing in it requires that your changes, or the work as a whole, stay under
zlib. So:

* **Yes, the Rust translation can be distributed under a different license**
  (MIT, Apache-2.0, MPL-2.0, GPL, a proprietary EULA, ...). The new license
  governs the new expression you added (the Rust-specific decisions, the
  idiomatic restructuring, the tests, the docs) and the terms under which you
  offer the combined work.
* **No, you cannot strip SDL's terms off the translated portions.** Whatever
  license you choose must be *compatible* with the zlib conditions, i.e. it
  must still (a) keep the zlib notice in the source distribution, (b) mark
  the work as an altered version of SDL, and (c) not claim the original is
  yours. Every mainstream license is compatible with these conditions, because
  they only require attribution and honesty about provenance.
* **You cannot stop anyone else from doing the same thing.** Because the
  translation is derived from zlib code, the parts that are SDL's expression
  remain available to everyone under zlib from the upstream project. Your new
  license only restricts *your* additions.

## What this repository does

This repository keeps the **zlib license** for everything, for three reasons:

1. It is the simplest honest position: one notice, one set of terms, no
   question about which lines are "SDL's" and which are "ours".
2. It keeps the door open for contributing fixes found during translation
   back upstream, and for pulling upstream changes down.
3. It matches the expectations of SDL's users; a `sdl3` crate with surprising
   terms would be a trap.

`LICENSE.txt` contains the unaltered SDL notice (condition 5), states that this
is an altered version (condition 4) and names the origin (condition 3). Each
source file repeats this in its header.

## If you want a different license later

You may relicense **your own contributions** at any time, since you hold their
copyright. To relicense the repository as a whole you would:

1. keep `LICENSE.txt`'s SDL notice and the "altered version" statement intact;
2. add your license text alongside it and state clearly which terms apply to
   which parts (or that the whole is offered under the new terms *in addition
   to* honoring the zlib conditions);
3. obtain agreement from every other contributor whose work is included, or
   have a contributor license agreement that allows it.

What you *cannot* do, whatever the chosen license, is remove the SDL notice or
present the work as an independent original.

## Clean-room alternative

The only way to produce a Rust SDL that is **not** a derivative work is a
clean-room implementation: one team writes a specification from the public
API documentation and observed behaviour, and a separate team that has never
read SDL's source implements it. API names and signatures themselves are at
most thinly protected (see *Google v. Oracle*, 2021), so a clean-room
`sdl3`-compatible library could be licensed however its authors like. That is a
different project from this one, which deliberately translates the real
implementation so that behaviour (and bugs, and quirks) match upstream.

## Third-party code inside SDL

Upstream SDL vendors a few pieces under other permissive licenses (for example
parts of `src/libm` from Sun's fdlibm, `SDL_qsort.c`, the HIDAPI code, parts of
`SDL_malloc.c` from dlmalloc, and the public-domain CRC/murmur3 snippets). When
those are translated, their original notices come along with them in the same
way; the per-file header will name the original license. All of them are
compatible with the zlib terms used here.
