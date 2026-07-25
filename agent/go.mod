module github.com/akiko99x/honey/agent

go 1.25.0

require (
	golang.org/x/crypto v0.54.0
	google.golang.org/grpc v1.82.1
	google.golang.org/protobuf v1.36.11
)

require (
	golang.org/x/net v0.56.0 // indirect
	golang.org/x/sys v0.47.0 // indirect
	golang.org/x/text v0.40.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20260414002931-afd174a4e478 // indirect
)

// after `buf generate ../proto`, run `go mod tidy` to fill in indirect deps + go.sum.
