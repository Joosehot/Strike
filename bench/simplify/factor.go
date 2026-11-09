package main

import "fmt"

// The C factor.c in Go: a*b + a*c + a*d - a*e is a*(b + c + d - e).
func main() {
	s := 0
	for i := 0; i < 200000000; i++ {
		a := i ^ 0x5bd1e995
		b := i >> 1
		c := i >> 2
		d := i >> 3
		e := i >> 4
		s = s + a*b + a*c + a*d - a*e
	}
	fmt.Println(s)
}
