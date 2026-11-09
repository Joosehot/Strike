// Strike's floats beyond single operations, written in the C that Strike
// itself compiles (no pointers, no unsigned numbers, no ?:). The lowering
// asks for these functions by name, as it asks for the ones in lib.rs, and
// they are compiled, branded and optimized as the program's own are.
//
// A float comes in as its 64 bits, a long long, as Strike IR carries it;
// a C float (and a Go float32) is held as the double of the same value.
//
// Printing is exact. A float is m * 2^e, which is a whole number of
// decimal digits times a power of ten: m * 2^e for e >= 0, and
// m * 5^-e / 10^-e for e < 0. Those digits are worked out with big numbers
// in limbs of base 10^9, and every format rounds them half to even, as
// C's printf (the UCRT's), Go's strconv and Python's repr do.
//
// It is real C as well: clang compiles it as it is, which is how it was
// first checked against printf, Go and Python.

#include <math.h>

// b = x (x >= 0), in limbs of base 10^9, the lowest first; returns how
// many limbs
long long big_set(long long b[], long long x) {
    b[0] = x % 1000000000;
    b[1] = x / 1000000000 % 1000000000;
    b[2] = x / 1000000000 / 1000000000;
    long long n = 3;
    while (n > 1 && b[n - 1] == 0) {
        n = n - 1;
    }
    return n;
}

// b = b * f, for 0 < f < 2^31 (a limb times f stays below 2^63); returns
// how many limbs
long long big_mul(long long b[], long long n, long long f) {
    long long carry = 0;
    for (long long i = 0; i < n; i++) {
        long long v = b[i] * f + carry;
        b[i] = v % 1000000000;
        carry = v / 1000000000;
    }
    while (carry > 0) {
        b[n] = carry % 1000000000;
        carry = carry / 1000000000;
        n = n + 1;
    }
    return n;
}

// x * 2^k (x >= 0) as decimal digits, the most significant first: the
// value is those digits times 10^-scale, and the scale is left in r[0].
// Returns how many digits.
long long exact(long long x, long long k, char d[], long long r[]) {
    long long b[100];
    long long n = big_set(b, x);
    long long scale = 0;
    if (k > 0) {
        while (k >= 30) {
            n = big_mul(b, n, 1073741824);
            k = k - 30;
        }
        long long p = 1;
        while (k > 0) {
            p = p * 2;
            k = k - 1;
        }
        n = big_mul(b, n, p);
    } else {
        scale = -k;
        long long j = -k;
        while (j >= 13) {
            n = big_mul(b, n, 1220703125);
            j = j - 13;
        }
        long long p = 1;
        while (j > 0) {
            p = p * 5;
            j = j - 1;
        }
        n = big_mul(b, n, p);
    }
    r[0] = scale;
    // the top limb without its leading zeros, then nine digits for each
    // of the others
    char t[12];
    long long tn = 0;
    long long top = b[n - 1];
    while (top > 0) {
        t[tn] = '0' + top % 10;
        top = top / 10;
        tn = tn + 1;
    }
    if (tn == 0) {
        t[0] = '0';
        tn = 1;
    }
    long long nd = 0;
    while (tn > 0) {
        tn = tn - 1;
        d[nd] = t[tn];
        nd = nd + 1;
    }
    for (long long i = n - 2; i >= 0; i--) {
        long long v = b[i];
        for (long long q = 8; q >= 0; q--) {
            d[nd + q] = '0' + v % 10;
            v = v / 10;
        }
        nd = nd + 9;
    }
    return nd;
}

// The first k digits, as a number, of n digits that stand o places to the
// right of the longest number they are compared with (o leading zeros).
long long prefix(char d[], long long n, long long o, long long k) {
    long long v = 0;
    for (long long i = 0; i < k; i++) {
        v = v * 10;
        if (i >= o) {
            v = v + d[i - o] - '0';
        }
    }
    return v;
}

// Is anything but zeros left after the first k digits?
long long rest_nonzero(char d[], long long n, long long o, long long k) {
    long long i = k - o;
    if (i < 0) {
        i = 0;
    }
    while (i < n) {
        if (d[i] != '0') {
            return 1;
        }
        i = i + 1;
    }
    return 0;
}

// How what is left after the first k digits compares with half a unit
// of the k-th: -1 below, 0 exactly half, 1 above.
long long rest_half(char d[], long long n, long long o, long long k) {
    long long i = k - o;
    if (i < 0 || i >= n) {
        return -1;
    }
    if (d[i] > '5') {
        return 1;
    }
    if (d[i] < '5') {
        return -1;
    }
    for (long long j = i + 1; j < n; j++) {
        if (d[j] != '0') {
            return 1;
        }
    }
    return 0;
}

// The shortest digits that read back as m * 2^e (m > 0) and, of those,
// the nearest to it (half to even): into out, with the place of the point
// in r[0] (the value is 0.digits * 10^r[0]). Returns how many digits.
//
// What reads back as m * 2^e lies between the halfway points to its
// neighbours, and the halfway points themselves read back as it when m is
// even (a tie goes to the even one). With lower_half the neighbour below
// is half as far (m is the lowest of its power of two). The three numbers
// are made exact in units of 2^(e-2), so they share one scale.
long long shortest(long long m, long long e, long long lower_half, char out[], long long r[]) {
    char dl[800];
    char dv[800];
    char dh[800];
    long long s[1];
    long long low = 4 * m - 2;
    if (lower_half) {
        low = 4 * m - 1;
    }
    long long nl = exact(low, e - 2, dl, s);
    long long nv = exact(4 * m, e - 2, dv, s);
    long long nh = exact(4 * m + 2, e - 2, dh, s);
    long long scale = s[0];
    long long inclusive = m % 2 == 0;
    long long ol = nh - nl;
    long long ov = nh - nv;
    // k digits from the top of the highest: the fewest that leave a
    // number between the two
    for (long long k = 1; k <= nh; k++) {
        long long lo = prefix(dl, nl, ol, k) + 1;
        if (inclusive && rest_nonzero(dl, nl, ol, k) == 0) {
            lo = lo - 1;
        }
        long long hi = prefix(dh, nh, 0, k);
        if (!inclusive && rest_nonzero(dh, nh, 0, k) == 0) {
            hi = hi - 1;
        }
        if (lo <= hi) {
            long long pv = prefix(dv, nv, ov, k);
            long long c = pv;
            long long h = rest_half(dv, nv, ov, k);
            if (h > 0 || (h == 0 && pv % 2 == 1)) {
                c = pv + 1;
            }
            if (c < lo) {
                c = lo;
            }
            if (c > hi) {
                c = hi;
            }
            // c times 10^(nh - k - scale)
            char t[24];
            long long tn = 0;
            while (c > 0) {
                t[tn] = '0' + c % 10;
                c = c / 10;
                tn = tn + 1;
            }
            r[0] = tn + nh - k - scale;
            long long nd = 0;
            while (tn > 0) {
                tn = tn - 1;
                out[nd] = t[tn];
                nd = nd + 1;
            }
            while (out[nd - 1] == '0') {
                nd = nd - 1;
            }
            return nd;
        }
    }
    return 0;
}

// The float's m, e (m * 2^e) and whether its neighbour below is half as
// far, into r[0], r[1], r[2] (finite, not zero). With f32 the value is a
// float32's, held as a double: its m and e are a float32's.
long long decode(long long bits, long long f32, long long r[]) {
    long long ex = bits >> 52 & 2047;
    long long frac = bits & 4503599627370495;
    long long m = frac;
    long long e = -1074;
    if (ex > 0) {
        m = frac + 4503599627370496;
        e = ex - 1075;
    }
    long long lower = frac == 0 && ex > 1;
    if (f32) {
        long long e2 = e + 29;
        if (e2 < -149) {
            e2 = -149;
        }
        m = m >> (e2 - e);
        e = e2;
        lower = m == 8388608 && e > -149;
    }
    r[0] = m;
    r[1] = e;
    r[2] = lower;
    return 0;
}

// d rounded to keep digits, half to even (trailing zeros already gone, so
// a 5 at the end is exactly half); the point moves in r[0] when 9s carry
// into a new digit. Returns how many digits are left (trailing zeros
// gone).
long long round_digits(char d[], long long nd, long long keep, long long r[]) {
    if (keep >= nd) {
        return nd;
    }
    if (keep < 0) {
        return 0;
    }
    long long up = 0;
    if (d[keep] > '5') {
        up = 1;
    }
    if (d[keep] == '5') {
        if (keep + 1 < nd) {
            up = 1;
        } else if (keep > 0 && (d[keep - 1] - '0') % 2 == 1) {
            up = 1;
        }
    }
    if (!up) {
        nd = keep;
        while (nd > 0 && d[nd - 1] == '0') {
            nd = nd - 1;
        }
        return nd;
    }
    long long i = keep - 1;
    while (i >= 0 && d[i] == '9') {
        i = i - 1;
    }
    if (i < 0) {
        d[0] = '1';
        r[0] = r[0] + 1;
        return 1;
    }
    d[i] = d[i] + 1;
    return i + 1;
}

// out[n..] = the word w (its letters as a number, the first lowest: "nan"
// is 'n' + 'a' * 256 + 'n' * 65536); returns the new length
long long word(char out[], long long n, long long w) {
    while (w > 0) {
        out[n] = w % 256;
        w = w / 256;
        n = n + 1;
    }
    return n;
}

// The digits (nd of them, the point at dp) in a format: 'e', 'E', 'f', or
// 'g', 'G' (%e or %f by the exponent), from out[n]; returns the new
// length. This is Go's fmtEFG (strconv), which C's printf agrees with for
// these formats once the digits are rounded as it rounds them.
long long efg(char out[], long long n, long long neg, char d[], long long dp, long long nd, long long prec, long long fmt, long long shortest) {
    if (fmt == 'g' || fmt == 'G') {
        long long eprec = prec;
        if (eprec > nd && nd >= dp) {
            eprec = nd;
        }
        // the shortest digits: %e from an exponent of 6 on
        if (shortest) {
            eprec = 6;
        }
        long long x = dp - 1;
        if (x < -4 || x >= eprec) {
            if (prec > nd) {
                prec = nd;
            }
            prec = prec - 1;
            fmt = fmt + 'e' - 'g';
        } else {
            if (prec > dp) {
                prec = nd;
            }
            prec = prec - dp;
            if (prec < 0) {
                prec = 0;
            }
            fmt = 'f';
        }
    }
    if (neg) {
        out[n] = '-';
        n = n + 1;
    }
    if (fmt == 'e' || fmt == 'E') {
        out[n] = '0';
        if (nd != 0) {
            out[n] = d[0];
        }
        n = n + 1;
        if (prec > 0) {
            out[n] = '.';
            n = n + 1;
            long long i = 1;
            while (i < nd && i <= prec) {
                out[n] = d[i];
                n = n + 1;
                i = i + 1;
            }
            while (i <= prec) {
                out[n] = '0';
                n = n + 1;
                i = i + 1;
            }
        }
        out[n] = fmt;
        n = n + 1;
        long long x = dp - 1;
        if (nd == 0) {
            x = 0;
        }
        out[n] = '+';
        if (x < 0) {
            out[n] = '-';
            x = -x;
        }
        n = n + 1;
        if (x >= 100) {
            out[n] = '0' + x / 100;
            n = n + 1;
        }
        out[n] = '0' + x / 10 % 10;
        out[n + 1] = '0' + x % 10;
        return n + 2;
    }
    // %f: the whole part, padded with zeros, then the fraction
    if (dp > 0) {
        for (long long i = 0; i < dp; i++) {
            out[n] = '0';
            if (i < nd) {
                out[n] = d[i];
            }
            n = n + 1;
        }
    } else {
        out[n] = '0';
        n = n + 1;
    }
    if (prec > 0) {
        out[n] = '.';
        n = n + 1;
        for (long long i = 0; i < prec; i++) {
            long long at = dp + i;
            out[n] = '0';
            if (at >= 0 && at < nd) {
                out[n] = d[at];
            }
            n = n + 1;
        }
    }
    return n;
}

// Python's repr of a float, into out; returns its length.
long long py_repr(long long bits, char out[]) {
    return py_repr_at(bits, out, 0);
}

// Python's math.sqrt of a negative number stops the program with this
// message; returns its length.
long long py_sqrt_error(long long bits, char out[]) {
    // "ValueError: expected a nonnegative input, got "
    long long n = word(out, 0, 32164449409065302);
    n = word(out, n, 33888186299215730);
    n = word(out, n, 9117586161886576);
    n = word(out, n, 28550393242722401);
    n = word(out, n, 9118758504522087);
    n = word(out, n, 9056077952872041);
    n = word(out, n, 544501607);
    n = py_repr_at(bits, out, n);
    out[n] = 10;
    return n + 1;
}

// Python's repr of a float, into out from out[n]; returns the new length.
long long py_repr_at(long long bits, char out[], long long n) {
    long long ex = bits >> 52 & 2047;
    long long frac = bits & 4503599627370495;
    if (ex == 2047 && frac != 0) {
        return word(out, n, 7233902);
    }
    if (bits < 0) {
        out[n] = '-';
        n = n + 1;
    }
    if (ex == 2047) {
        return word(out, n, 6712937);
    }
    if (ex == 0 && frac == 0) {
        return word(out, n, 3157552);
    }
    char d[24];
    long long r[3];
    decode(bits, 0, r);
    long long nd = shortest(r[0], r[1], r[2], d, r);
    long long dp = r[0];
    // the point in its place from 1e-4 up to below 1e16, else an exponent
    if (dp > -4 && dp <= 16) {
        if (dp <= 0) {
            n = word(out, n, 11824);
            for (long long i = 0; i < -dp; i++) {
                out[n] = '0';
                n = n + 1;
            }
            for (long long i = 0; i < nd; i++) {
                out[n] = d[i];
                n = n + 1;
            }
            return n;
        }
        for (long long i = 0; i < dp; i++) {
            out[n] = '0';
            if (i < nd) {
                out[n] = d[i];
            }
            n = n + 1;
        }
        out[n] = '.';
        n = n + 1;
        if (nd <= dp) {
            out[n] = '0';
            return n + 1;
        }
        for (long long i = dp; i < nd; i++) {
            out[n] = d[i];
            n = n + 1;
        }
        return n;
    }
    return efg(out, n, 0, d, dp, nd, nd - 1, 'e', 0);
}

// What C's printf (the UCRT's) prints for a NaN or an infinity in the
// format fmt, from out[n] (the sign already there for an infinity).
long long c_special(char out[], long long n, long long bits, long long fmt) {
    long long upper = fmt == 'E' || fmt == 'G';
    long long frac = bits & 4503599627370495;
    if (frac == 0) {
        if (upper) {
            return word(out, n, 4607561);
        }
        return word(out, n, 6712937);
    }
    if (upper) {
        n = word(out, n, 5128526);
    } else {
        n = word(out, n, 7233902);
    }
    // a signalling NaN (the quiet bit clear): (snan)
    if (frac < 2251799813685248) {
        if (upper) {
            n = word(out, n, 5133096);
            return word(out, n, 2707009);
        }
        n = word(out, n, 7238440);
        return word(out, n, 2715233);
    }
    // the NaN the machine makes for an invalid operation (negative, only
    // the quiet bit set): (ind)
    if (frac == 2251799813685248 && bits < 0) {
        if (upper) {
            n = word(out, n, 5130536);
            return word(out, n, 10564);
        }
        n = word(out, n, 7235880);
        return word(out, n, 10596);
    }
    return n;
}

// C's printf and Go's Printf of a float in the format fmt ('e', 'E', 'f',
// 'g', 'G') with prec digits; prec < 0 is Go's shortest (%v, %g), f32
// then the shortest of a float32. go: the words Go prints for NaN and the
// infinities; else the UCRT's. Into out; returns the length.
long long fmt_float(long long bits, long long f32, long long fmt, long long prec, long long go, char out[]) {
    long long n = 0;
    long long neg = bits < 0;
    long long ex = bits >> 52 & 2047;
    long long frac = bits & 4503599627370495;
    if (ex == 2047) {
        if (go) {
            if (frac != 0) {
                return word(out, 0, 5136718);
            }
            out[0] = '+';
            if (neg) {
                out[0] = '-';
            }
            return word(out, 1, 6712905);
        }
        if (neg) {
            out[0] = '-';
            n = 1;
        }
        return c_special(out, n, bits, fmt);
    }
    char d[800];
    long long r[3];
    long long nd = 0;
    long long dp = 0;
    long long shortest_ = prec < 0;
    if (ex != 0 || frac != 0) {
        decode(bits, f32 && prec < 0, r);
        if (prec < 0) {
            nd = shortest(r[0], r[1], r[2], d, r);
            dp = r[0];
            prec = nd;
            if (fmt == 'e' || fmt == 'E') {
                prec = nd - 1;
            }
            if (fmt == 'f') {
                prec = nd - dp;
                if (prec < 0) {
                    prec = 0;
                }
            }
        } else {
            nd = exact(r[0], r[1], d, r);
            dp = nd - r[0];
            while (d[nd - 1] == '0') {
                nd = nd - 1;
            }
            long long keep = prec;
            if (fmt == 'e' || fmt == 'E') {
                keep = prec + 1;
            } else if (fmt == 'f') {
                keep = dp + prec;
            } else if (prec == 0) {
                prec = 1;
                keep = 1;
            }
            r[0] = dp;
            nd = round_digits(d, nd, keep, r);
            dp = r[0];
        }
    } else if (prec == 0 && (fmt == 'g' || fmt == 'G')) {
        prec = 1;
    }
    return efg(out, 0, neg, d, dp, nd, prec, fmt, shortest_);
}

// Python's comparison of a float and a whole number: exact, not by making
// the number a float. -1, 0 or 1 as f is below, equal to or above i, and
// 2 when f is NaN.
long long py_cmp(double f, long long i) {
    if (f != f) {
        return 2;
    }
    if (i >= -9007199254740992 && i <= 9007199254740992) {
        double d = i;
        if (f < d) {
            return -1;
        }
        if (f > d) {
            return 1;
        }
        return 0;
    }
    if (f >= 9223372036854775808.0) {
        return 1;
    }
    if (f < -9223372036854775808.0) {
        return -1;
    }
    // f is cut toward zero; where it equals i, |i| > 2^53, so f has no
    // fraction and is i
    long long t = f;
    if (t < i) {
        return -1;
    }
    if (t > i) {
        return 1;
    }
    return 0;
}

// The float's floor (a whole number already, NaN and the infinities stay).
double floor_of(double x) {
    if (!(fabs(x) < 4503599627370496.0)) {
        return x;
    }
    long long k = x;
    double t = k;
    if (t > x) {
        t = t - 1.0;
    }
    return t;
}

// Is the sign of x negative (also for -0.0)?
long long negative(double x) {
    return x < 0.0 || (x == 0.0 && 1.0 / x < 0.0);
}

// Python's float a // b (b is not zero: the caller has checked), as
// CPython's float_floor_div works it out.
double py_floordiv(double vx, double wx) {
    double mod = fmod(vx, wx);
    double div = (vx - mod) / wx;
    if (mod != 0.0) {
        if ((wx < 0.0) != (mod < 0.0)) {
            div = div - 1.0;
        }
    }
    if (div != 0.0) {
        double fd = floor_of(div);
        if (div - fd > 0.5) {
            fd = fd + 1.0;
        }
        return fd;
    }
    if (negative(vx / wx)) {
        return -0.0;
    }
    return 0.0;
}

// Python's float a % b (b is not zero), as CPython's float_rem.
double py_mod(double vx, double wx) {
    double mod = fmod(vx, wx);
    if (mod != 0.0) {
        if ((wx < 0.0) != (mod < 0.0)) {
            mod = mod + wx;
        }
        return mod;
    }
    if (wx < 0.0) {
        return -0.0;
    }
    return 0.0;
}

int main() {
    return 0;
}
