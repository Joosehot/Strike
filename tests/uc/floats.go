package main

import (
	"fmt"
	"math"
)

// floats as Go has them: float64 and float32, constants that take the type
// they meet, explicit conversions, Println's %v and Printf's verbs

var total float64
var scale float32 = 1.5

func hyp(a, b float64) float64 {
	return math.Sqrt(a*a + b*b)
}

func mean(xs []float64) float64 {
	s := 0.0
	for _, v := range xs {
		s += v
	}
	return s / float64(len(xs))
}

func half32(x float32) float32 {
	return x / 2
}

func add(x float64) {
	total = total + x
}

func main() {
	fmt.Println(0.1, 0.25, 1.0, 100000.0, 1e6, 1234567.0, 1e20, 1e21, 1e-4, 1e-5)
	fmt.Println(2.5e-7, 5e-324, 1.7976931348623157e308, 123456789.0, 1.0/3.0)
	var z float64
	fmt.Println(-z, z, 1/z, -1/z, z/z, -0.0)

	x := 1.5
	y := x*2 + 0.1
	fmt.Println(x, y, -x, x/3, x-y, x*x*x)
	i := 7
	fmt.Println(float64(i)/2, int(x*3), int(-x*2), i*2.0, float64(i)*0.1)

	var f float32 = 0.1
	g := float32(16777217)
	fmt.Println(f, f*3, f+0.2, g, float64(f), half32(3), float32(0.1)+float32(0.2))
	fmt.Println(scale*2, float32(1)/3, float64(float32(1)/3), float32(x))

	fmt.Println(hyp(3, 4), hyp(1, 1), math.Sqrt(2), math.Abs(-2.5), math.Sqrt(-1))
	xs := []float64{1, 2.5, -0.75, 1e30}
	xs = append(xs, 0.125)
	xs[0] = xs[0] * 2
	xs[1] += 1
	fmt.Println(xs, len(xs), mean(xs))
	var arr [3]float32
	arr[1] = 0.5
	arr[2] = f
	fmt.Println(arr, []float32{1.25, 3})
	m := make([]float64, 3)
	m[2] = math.Sqrt(m[1] + 9)
	fmt.Println(m)

	for k := 0; k < 10; k++ {
		add(0.1)
	}
	fmt.Println(total, total == 1, 1-total)

	fmt.Printf("%f %e %g %v\n", x, x, x/3, y)
	fmt.Printf("%.0f %.0f %.1f %.2f %.3e %.10g\n", 0.5, 2.5, 0.25, 1.005, 12345.678, 1.0/3)
	fmt.Printf("%E %G %g %g %g\n", 1e-7, 1e-7, 1e21, 123456789.0, float32(0.1))
	fmt.Printf("%f %e %g %v %.2f\n", z/z, -1/z, 1/z, -z, 1e300)

	acc := 0.0
	for k := 1; k <= 1000; k++ {
		acc += 1.0 / float64(k*k)
	}
	fmt.Println(acc, acc*6, acc < 1.65, acc >= 2)
	n := 2.0
	for k := 0; k < 30; k++ {
		n = (n + 2/n) / 2
	}
	fmt.Println(n, n*n, n*n == 2)
}
