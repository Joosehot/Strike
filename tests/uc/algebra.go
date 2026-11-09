package main

// The same expressions as algebra.c, in Go: its ints wrap at 64 bits, so
// the numbers may be as large as they like; int32 wraps at 32.

import "fmt"

var g [256]int

func poly(x int) int {
	return 3*x*x*x + 5*x*x + 7*x + 11
}

func factor(a int, b int, c int, d int) int {
	return a*b + a*c - a*d + b*c*3 - c*b
}

func consts(x int, y int) int {
	return x*9 + y*5 + x*3 - y*7 + (x << 3) - (y << 1) + x*2 + 17
}

func atoms(a int, b int, c int) int {
	q := (a*b+a*c)/3 + (a*b-a*c)%5
	r := 0
	if a*3+b > c*2 {
		r = r + 1
	}
	if a*a-b*b == (a+b)*(a-b) {
		r = r + 1
	}
	return q*7 + r*(a+1) - (a * b >> 2) + ^a*3 + -b*4
}

func narrow(a int32, b int32) int32 {
	x := a*b*3 + a*5 - b*7 + (a^b)*9
	return x*x*7 + x*3 - (x/7)*3
}

func rows(n int, m int) int {
	s := 0
	for i := 0; i < n; i++ {
		for j := 0; j < m; j++ {
			s = s + g[(i*m+j)&255]*(j+1) + (i*m+j)*8 + i*3
		}
	}
	return s
}

func main() {
	for k := 0; k < 256; k++ {
		g[k] = (k*37+11)%101 - 50
	}
	s := 0
	for x := -20; x <= 20; x++ {
		s = s + poly(x)
		fmt.Println(poly(x), factor(x, x+3, 7-x, x*2), consts(x, 3-x))
		fmt.Println(atoms(x, x*3-5, 11-x), narrow(int32(x*101), int32(x-7)))
	}
	big := 1 << 40
	fmt.Println(s, rows(9, 13), rows(16, 16), poly(big+12345), factor(big, big*3+1, -big, 77))
}
