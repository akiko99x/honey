module github.com/akiko99x/honey/agent

go 1.23

require (
	google.golang.org/grpc v1.67.1
	google.golang.org/protobuf v1.36.11
)

require (
	golang.org/x/net v0.28.0 // indirect
	golang.org/x/sys v0.24.0 // indirect
	golang.org/x/text v0.17.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20240814211410-ddb44dafa142 // indirect
)

// after `buf generate ../proto`, run `go mod tidy` to fill in indirect deps + go.sum.
