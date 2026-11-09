package main

import "fmt"

func main() {
	x := 5
	var y int32 = 1
	a := []int{12, 10}
	x <<= 3
	x >>= 1
	x &= 0x1E
	x |= 64
	x ^= 5
	y <<= 30
	y <<= 1
	y ^= 9
	a[0] ^= 7
	a[1] <<= 2
	fmt.Println(x, y, a)
}

