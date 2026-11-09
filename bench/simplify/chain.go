package main

import "fmt"

// The C chain.c in Go: a recurrence of polynomials.
func main() {
	x := 1
	y := 2
	for i := 0; i < 100000000; i++ {
		x = 3*x*x*y + 5*x*y + 7*y + i
		y = y*9 + x*5 - y*3 + 1
	}
	fmt.Println(x, y)
}
