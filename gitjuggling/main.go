package main

import (
	"os"

	"dev.rischmann.fr/mytools/gitjuggling/cmd"
)

func main() {
	if err := cmd.Execute(); err != nil {
		os.Exit(1)
	}
}
