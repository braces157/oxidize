//! Myers diff algorithm computing the shortest edit script between two sequences.

/// A single edit operation in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp<'a> {
    /// Line unchanged in both versions.
    Keep(&'a str),
    /// Line inserted in new version.
    Insert(&'a str),
    /// Line deleted from old version.
    Delete(&'a str),
}

/// Computes the Myers diff between two slices of line strings.
pub fn myers_diff<'a>(a: &'a [&'a str], b: &'a [&'a str]) -> Vec<DiffOp<'a>> {
    let n = a.len();
    let m = b.len();
    let max = n + m;

    if max == 0 {
        return Vec::new();
    }

    let mut v = vec![0isize; 2 * max + 1];
    let offset = max as isize;

    let mut trace = Vec::new();

    for d in 0..=max {
        trace.push(v.clone());
        let mut k = -(d as isize);
        while k <= d as isize {
            let k_idx = (k + offset) as usize;
            let mut x = if k == -(d as isize) || (k != d as isize && v[k_idx - 1] < v[k_idx + 1]) {
                v[k_idx + 1]
            } else {
                v[k_idx - 1] + 1
            };

            let mut y = x - k;

            while (x as usize) < n && (y as usize) < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }

            v[k_idx] = x;

            if x >= n as isize && y >= m as isize {
                return backtrack(&trace, a, b, offset);
            }

            k += 2;
        }
    }

    backtrack(&trace, a, b, offset)
}

fn backtrack<'a>(
    trace: &[Vec<isize>],
    a: &'a [&'a str],
    b: &'a [&'a str],
    offset: isize,
) -> Vec<DiffOp<'a>> {
    let mut x = a.len() as isize;
    let mut y = b.len() as isize;
    let mut ops = Vec::new();

    for (d, v) in trace.iter().enumerate().rev() {
        let k = x - y;
        let k_idx = (k + offset) as usize;

        let prev_k = if k == -(d as isize) || (k != d as isize && v[k_idx - 1] < v[k_idx + 1]) {
            k + 1
        } else {
            k - 1
        };

        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            ops.push(DiffOp::Keep(a[x as usize]));
        }

        if d > 0 {
            if x == prev_x {
                y -= 1;
                ops.push(DiffOp::Insert(b[y as usize]));
            } else {
                x -= 1;
                ops.push(DiffOp::Delete(a[x as usize]));
            }
        }
    }

    ops.reverse();
    ops
}
