#[derive(Debug, Clone, Copy, PartialEq, Eq)]

#[allow(unused)]
pub enum OverflowStrategy {
    DropNewest,
    DropOldest,
}

pub struct RingBuffer<T, const N: usize> {
    data: [Option<T>; N],
    head: usize,
    tail: usize,
    size: usize,
    strategy: OverflowStrategy,
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    pub const fn new(strategy: OverflowStrategy) -> Self {
        assert!(N > 0, "Buffer size must be greater than zero");
        Self {
            data: [None; N],
            head: 0,
            tail: 0,
            size: 0,
            strategy,
        }
    }

    pub fn push(&mut self, item: T) -> Result<(), T> {
        if self.is_full() {
            match self.strategy {
                OverflowStrategy::DropNewest => {
                    // Reject the new item
                    return Err(item);
                }
                OverflowStrategy::DropOldest => {
                    // Advance head to "delete" the oldest item
                    self.head = (self.head + 1) % N;
                    self.size -= 1;
                }
            }
        }

        self.data[self.tail] = Some(item);
        self.tail = (self.tail + 1) % N;
        self.size += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let item = self.data[self.head].take();
        self.head = (self.head + 1) % N;
        self.size -= 1;
        item
    }

    pub fn is_full(&self) -> bool {
        self.size == N
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}