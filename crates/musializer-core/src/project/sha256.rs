//! Content-addressed asset identity: SHA-256.
//!
//! **Owner: Agent B.** Ported by hand from `../musializer/src/sha256.c`.
//!
//! Hand-rolled deliberately, for two reasons. The C does it by hand too, so a
//! line-by-line port is checkable against the oracle; and `musializer-core` has
//! no `sha2` dependency yet, so a hand port is the only thing that compiles
//! today. If `sha2` is added later this module should keep its API and delegate
//! — the *hex spelling* (lowercase, 64 characters, no prefix) is a `.musi`
//! compatibility surface, not an implementation detail.
//!
//! `sha256_file` (`sha256.c:206-242`) is deliberately **not** here: it opens a
//! file, and this crate has no filesystem. Stream a file through
//! [`Sha256::update`] from `musializer-runtime` instead — the C chunk size is
//! 64 KiB (`sha256.c:214`), and nothing about the digest depends on it.

/// Digest length in bytes (`sha256.h`).
pub const DIGEST_SIZE: usize = 32;
/// Hex digest length in characters, without C's trailing NUL.
pub const HEX_SIZE: usize = 64;

#[rustfmt::skip]
const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[rustfmt::skip]
const INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// A streaming SHA-256 (`Sha256` in `sha256.h`).
///
/// C tracks a `finalized` flag and returns `false` from a second `sha256_final`
/// (`sha256.c:145`). Rust expresses the same rule in the type system:
/// [`Sha256::finalize`] takes `self` by value, so a finalized hasher cannot be
/// used again at all. That is why the C tests for "update after final fails"
/// have no Rust counterpart — they check something the compiler now checks.
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_size: usize,
    total_size: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// `sha256_init` (`sha256.c:97-108`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: INITIAL_STATE,
            buffer: [0; 64],
            buffer_size: 0,
            total_size: 0,
        }
    }

    /// `sha256_compress` (`sha256.c:46-95`).
    fn compress(&mut self, block: &[u8; 64]) {
        let mut schedule = [0u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            let at = index * 4;
            *word = u32::from_be_bytes([block[at], block[at + 1], block[at + 2], block[at + 3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(ROUND_CONSTANTS[index])
                .wrapping_add(schedule[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    /// `sha256_update` (`sha256.c:110-141`).
    pub fn update(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let mut bytes = data;
        self.total_size = self.total_size.wrapping_add(bytes.len() as u64);

        if self.buffer_size != 0 {
            let available = self.buffer.len() - self.buffer_size;
            let take = bytes.len().min(available);
            self.buffer[self.buffer_size..self.buffer_size + take].copy_from_slice(&bytes[..take]);
            self.buffer_size += take;
            bytes = &bytes[take..];
            if self.buffer_size == self.buffer.len() {
                let block = self.buffer;
                self.compress(&block);
                self.buffer_size = 0;
            }
        }

        while bytes.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&bytes[..64]);
            self.compress(&block);
            bytes = &bytes[64..];
        }
        if !bytes.is_empty() {
            self.buffer[..bytes.len()].copy_from_slice(bytes);
            self.buffer_size = bytes.len();
        }
    }

    /// `sha256_final` (`sha256.c:143-168`).
    #[must_use]
    pub fn finalize(mut self) -> [u8; DIGEST_SIZE] {
        let bit_size = self.total_size.wrapping_mul(8);
        let mut used = self.buffer_size;
        self.buffer[used] = 0x80;
        used += 1;

        if used > 56 {
            for slot in self.buffer[used..].iter_mut() {
                *slot = 0;
            }
            let block = self.buffer;
            self.compress(&block);
            used = 0;
        }
        for slot in self.buffer[used..56].iter_mut() {
            *slot = 0;
        }
        self.buffer[56..64].copy_from_slice(&bit_size.to_be_bytes());
        let block = self.buffer;
        self.compress(&block);

        let mut digest = [0u8; DIGEST_SIZE];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

/// `sha256_digest` (`sha256.c:170-180`).
#[must_use]
pub fn digest(data: &[u8]) -> [u8; DIGEST_SIZE] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

/// `sha256_hex` (`sha256.c:182-193`): lowercase hex, no prefix, exactly 64
/// characters.
///
/// The spelling is the `.musi` contract (`$defs/sha256` is `^[0-9a-f]{64}$`), so
/// uppercase output would be a compatibility break, not a cosmetic difference.
#[must_use]
pub fn hex(digest: &[u8; DIGEST_SIZE]) -> String {
    const ALPHABET: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(HEX_SIZE);
    for byte in digest {
        out.push(ALPHABET[(byte >> 4) as usize] as char);
        out.push(ALPHABET[(byte & 0x0f) as usize] as char);
    }
    out
}

/// `sha256_digest_hex` (`sha256.c:195-204`).
#[must_use]
pub fn digest_hex(data: &[u8]) -> String {
    hex(&digest(data))
}

/// True for the digest spelling the schema accepts: exactly 64 lowercase hex
/// digits (`project.c:39-47`, `project-v1.schema.json` `$defs/sha256`).
///
/// Uppercase is rejected on purpose so one digest has one spelling.
#[must_use]
pub fn is_hex_digest(value: &str) -> bool {
    value.len() == HEX_SIZE
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase() && byte <= b'f')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors copied from `../musializer/tests/test_sha256.c:13-23`.
    #[test]
    fn matches_standard_known_vectors() {
        assert_eq!(
            digest_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// `test_sha256.c:25-43`: one byte at a time must equal the one-shot digest.
    #[test]
    fn incremental_updates_match_one_shot() {
        const MESSAGE: &[u8] = b"incremental hashing across awkward boundaries";
        let mut hasher = Sha256::new();
        for byte in MESSAGE {
            hasher.update(&[*byte]);
        }
        assert_eq!(hex(&hasher.finalize()), digest_hex(MESSAGE));
    }

    /// `test_sha256.c:45-58`: the million-`a` vector, which is the only test that
    /// exercises a 64-bit length field beyond a single block count.
    #[test]
    fn handles_the_million_a_vector() {
        let block = [b'a'; 1000];
        let mut hasher = Sha256::new();
        for _ in 0..1000 {
            hasher.update(&block);
        }
        assert_eq!(
            hex(&hasher.finalize()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn every_padding_branch_agrees_with_streaming() {
        // 55, 56, 63, 64 and 65 bytes cover each branch of `sha256_final`: fits
        // in the tail, spills into a second block, exactly full, one past full.
        for length in [0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 128] {
            let data = vec![b'z'; length];
            let expected = digest(&data);
            for chunk in [1usize, 7, 63, 64, 65] {
                let mut hasher = Sha256::new();
                for piece in data.chunks(chunk) {
                    hasher.update(piece);
                }
                assert_eq!(hasher.finalize(), expected, "length {length} chunk {chunk}");
            }
        }
    }

    /// `sha256.c:114`: a zero-length update is a no-op, not an error.
    #[test]
    fn empty_updates_do_not_change_the_digest() {
        let mut hasher = Sha256::new();
        hasher.update(b"");
        hasher.update(b"abc");
        hasher.update(b"");
        assert_eq!(hex(&hasher.finalize()), digest_hex(b"abc"));
    }

    #[test]
    fn hex_digests_are_recognised_by_spelling_only() {
        assert!(is_hex_digest(&digest_hex(b"abc")));
        assert!(!is_hex_digest(&digest_hex(b"abc").to_uppercase()));
        assert!(!is_hex_digest(""));
        assert!(!is_hex_digest(&"a".repeat(63)));
        assert!(!is_hex_digest(&"a".repeat(65)));
        assert!(!is_hex_digest(&"g".repeat(64)));
    }
}
