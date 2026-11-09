package main

import (
	"example.com/multi/calc"
	"example.com/multi/geo"
	"fmt"
)

// given after calc's own variables, which it reads
var base = calc.Scale * 2

func main() {
	fmt.Println(calc.Add(2, 3))
	fmt.Println(twice(calc.Square(4)))
	fmt.Println(geo.Area(3, 4), base)
	calc.Total = 7
	calc.Bump()
	fmt.Println(calc.Total, calc.Count())
	fmt.Println(counter)
}
