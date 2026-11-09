package main

// The same work as values.c in Go: its ints wrap at 64 bits and int32 at
// 32, so here the numbers may be as large as they like.

import "fmt"

func halves(n int) int {
	s := 0
	for i := 0; i < n; i++ {
		a := i ^ 1540483477
		b := i >> 1
		c := i >> 2
		d := (i + 1) >> 3
		s = s + a*b + a*c - a*d + ((i + 1) & 6) + ((i + 3) ^ 5)
	}
	return s
}

func shared(x int, y int, n int) int {
	s := 0
	t := 0
	for i := 0; i < n; i++ {
		p := (x+i)*(y-i) + x*3
		q := (x+i)*(y-i) - y*5
		s = s + p*2 + (x+i)*(y-i)
		t = t + q
		p = (x+i)*(y-i) + x*3
		s = s - p
	}
	return s*7 + t
}

func narrow(n int32) int32 {
	var s int32 = 0
	for i := int32(0); i < 1000; i++ {
		a := (i&255)*1000 + i*3
		b := i*i - 500*i
		s = s*31 + a + b
	}
	for j := int32(0); j < n; j++ {
		s = s + j*2 + (j >> 1)
	}
	return s
}

func main() {
	fmt.Println(halves(1000), halves(1001), halves(7))
	fmt.Println(shared(12, -7, 100), shared(-3, 40, 51), shared(1<<40, 3<<35, 77))
	fmt.Println(narrow(10), narrow(999))
}
