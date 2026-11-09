package main

import "fmt"

// The C poly.c in Go: Horner's form takes two multiplications of six.
func main() {
	s := 0
	for i := 0; i < 200000000; i++ {
		x := i ^ (i >> 3)
		s = s + 3*x*x*x + 5*x*x + 7*x + 11
	}
	fmt.Println(s)
}
