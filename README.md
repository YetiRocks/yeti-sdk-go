# yeti-sdk-go

The Go binding for the [Yeti](https://yetirocks.com) SDK: the front-door client
(module `github.com/yetirocks/yeti-sdk-go`, package `yetisdk`), cgo over a C ABI
on the same Rust core every Yeti SDK wraps.

```go
import yetisdk "github.com/yetirocks/yeti-sdk-go"

fd, err := yetisdk.ConfigureFrontDoor("wss://example.com/yeti-sdk/ws", "example.com", apiKey, "")
defer fd.Close()
todos := fd.Table("todo-demo", "api/Todo")
row, err := todos.Put("t1", []byte(`{"id":"t1","title":"hello"}`))
```

Build with `CGO_ENABLED=1` and a C toolchain.

## Where the code comes from

This repository is a release mirror. The source lives in the Yeti repository
under `sdks/yeti-sdk-go/`, and each release is a tag `vX.Y.Z` whose commit
carries the Go package together with the prebuilt static archive for every
supported platform under `lib/<goos>_<goarch>/` and the generated C header.
`go get github.com/yetirocks/yeti-sdk-go@vX.Y.Z` fetches that tag; `main`
holds the Go source only and does not link on its own.

## License

Apache-2.0; see [LICENSE](LICENSE).
