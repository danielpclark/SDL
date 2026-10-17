/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluid_ringbuffer.c and
// fluid_ringbuffer.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

/*
 * Josh Green <josh@resonance.org>
 * 2009-05-28
 */

/// Translation of `fluid_ringbuffer_t`: lockless event queue instance (here
/// used from one thread, so with plain counters).
#[derive(Debug)]
pub(crate) struct FluidRingbuffer<T> {
    array: Vec<T>,     /*< Queue array of arbitrary size elements */
    totalcount: usize, /*< Total count of elements in array */
    count: usize,      /*< Current count of elements */
    in_: usize,        /*< Index in queue to store next pushed element */
    out: usize,        /*< Index in queue of next popped element */
}

impl<T: Default> FluidRingbuffer<T> {
    /// Translation of `new_fluid_ringbuffer()`: create a lock free queue
    /// with a fixed maximum count and size of elements.
    ///
    /// Lockless FIFO queues don't use any locking mechanisms and can therefore be
    /// advantageous in certain situations, such as passing data between a lower
    /// priority thread and a higher "real time" thread, without potential lock
    /// contention which could stall the high priority thread.  Note that there may
    /// only be one producer thread and one consumer thread.
    pub(crate) fn new(count: usize) -> Option<Self> {
        if count == 0 {
            return None;
        }

        let mut array = Vec::new();
        array.try_reserve_exact(count).ok()?;
        /* Clear array, in case dynamic pointer reclaiming is being done */
        array.resize_with(count, T::default);

        Some(FluidRingbuffer {
            array,
            totalcount: count,
            count: 0,
            in_: 0,
            out: 0,
        })
    }
}

impl<T> FluidRingbuffer<T> {
    /// Translation of `fluid_ringbuffer_get_inptr()`: get pointer to next
    /// input array element in queue.
    ///
    /// @param offset Normally zero, or more if you need to push several items at once
    /// @return Pointer to array element in queue to store data to or NULL if queue is full
    ///
    /// This function along with fluid_ringbuffer_next_inptr() form a queue "push"
    /// operation and is split into 2 functions to avoid an element copy.  Note that
    /// the returned array element pointer may contain the data of a previous element
    /// if the queue has wrapped around.  This can be used to reclaim pointers to
    /// allocated memory, etc.
    #[inline]
    pub(crate) fn get_inptr(&mut self, offset: usize) -> Option<&mut T> {
        if self.count + offset >= self.totalcount {
            None
        } else {
            let i = (self.in_ + offset) % self.totalcount;
            Some(&mut self.array[i])
        }
    }

    /// Translation of `fluid_ringbuffer_next_inptr()`: advance the input
    /// queue index to complete a "push" operation.
    ///
    /// @param count Normally one, or more if you need to push several items at once
    #[inline]
    pub(crate) fn next_inptr(&mut self, count: usize) {
        self.count += count;
        self.in_ += count;
        if self.in_ >= self.totalcount {
            self.in_ -= self.totalcount;
        }
    }

    /// Translation of `fluid_ringbuffer_get_count()`: get amount of items
    /// currently in queue.
    #[inline]
    pub(crate) fn get_count(&self) -> usize {
        self.count
    }

    /// Translation of `fluid_ringbuffer_get_outptr()`: get pointer to next
    /// output array element in queue.
    ///
    /// @return Pointer to array element data in the queue or NULL if empty, can only
    ///   be used up until fluid_ringbuffer_next_outptr() is called.
    #[inline]
    pub(crate) fn get_outptr(&mut self) -> Option<&mut T> {
        if self.get_count() == 0 {
            None
        } else {
            Some(&mut self.array[self.out])
        }
    }

    /// Translation of `fluid_ringbuffer_next_outptr()`: advance the output
    /// queue index to complete a "pop" operation.
    #[inline]
    pub(crate) fn next_outptr(&mut self) {
        self.count -= 1;
        self.out += 1;
        if self.out == self.totalcount {
            self.out = 0;
        }
    }
}
