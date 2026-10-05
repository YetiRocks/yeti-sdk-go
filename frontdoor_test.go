// frontdoor_test.go — front-door cgo SDK acceptance (YTC-1122).
//
// Drives the front-door SDK client — surfaced through the C ABI — from a Go
// process against a RUNNING yeti node's authenticated /yeti-sdk/ws front door.
// This is the SDK-real path (ADR-030), NOT the dead back-door host-RPC listener
// and NOT a stock-REST shortcut: every op below rides the front-door channel the
// binding negotiated.
//
// The live node is stood up by sdk-cells fixture (see
// sdk-cells cell), which exports YETI_FD_* into the environment. Without
// them the test skips, so `go test ./...` stays runnable standalone.

package yetisdk

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"testing"
	"time"
)

// todo is the todo-demo reference record: the same id/title/done shape every
// cell of the 7-language showcase drives.
type todo struct {
	ID    string `json:"id"`
	Title string `json:"title"`
	Done  bool   `json:"done"`
}

func TestFrontDoorRoundTrip(t *testing.T) {
	url := os.Getenv("YETI_FD_URL")
	caPath := os.Getenv("YETI_FD_CA")
	if url == "" || caPath == "" {
		t.Skip("YETI_FD_URL / YETI_FD_CA unset — run sdk-cells cell for the live-node fixture")
	}
	host := envOr("YETI_FD_HOST", "localhost")
	app := envOr("YETI_FD_APP", "todo-demo")
	table := envOr("YETI_FD_TABLE", "api/Todo")

	caPem, err := os.ReadFile(caPath)
	if err != nil {
		t.Fatalf("read CA pem %s: %v", caPath, err)
	}

	// Connect over wss trusting the node's dev CA (anonymous — public ops only).
	fd, err := ConfigureFrontDoorPinned(url, host, string(caPem), "", "")
	if err != nil {
		t.Fatalf("ConfigureFrontDoorPinned: %v", err)
	}
	defer fd.Close()

	transport, err := fd.ActiveTransport()
	if err != nil {
		t.Fatalf("ActiveTransport: %v", err)
	}
	// ASSERT the rung. Transport::Auto falls silently down the ladder to
	// REST+SSE, so a merely-logged transport lets a run "pass" having never
	// exercised the WebSocket front door this cell claims to prove.
	if transport != "WebSocket" {
		t.Fatalf("the ladder fell back to %q; this run does not prove the primary rung", transport)
	}
	t.Logf("PASS front door established on the asserted WebSocket rung (activeTransport=%s)", transport)

	token, err := fd.SessionToken()
	if err != nil {
		t.Fatalf("SessionToken: %v", err)
	}
	if token == "" {
		t.Fatal("fresh connection did not receive a host-minted session token")
	}
	t.Log("PASS fresh connection receives a host-minted session token")

	tbl := fd.Table(app, table)

	// CREATE — the todo-demo "add a todo" op.
	key := fmt.Sprintf("go-%d", time.Now().UnixNano())
	body, err := json.Marshal(todo{ID: key, Title: "walk the yeti", Done: false})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	putBytes, err := tbl.Put(key, body)
	if err != nil {
		t.Fatalf("Put: %v", err)
	}
	var created todo
	if err := json.Unmarshal(putBytes, &created); err != nil {
		t.Fatalf("unmarshal put echo (%s): %v", putBytes, err)
	}
	if created.ID != key || created.Title != "walk the yeti" || created.Done {
		t.Fatalf("put echoed an unexpected row: %+v", created)
	}
	t.Log("PASS create echoes the persisted row")

	// GET — the row round-trips over the front door.
	got, found, err := tbl.Get(key)
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if !found {
		t.Fatal("Get missed a row that was just written")
	}
	var fetched todo
	if err := json.Unmarshal(got, &fetched); err != nil {
		t.Fatalf("unmarshal get (%s): %v", got, err)
	}
	if fetched.Title != "walk the yeti" || fetched.Done {
		t.Fatalf("get returned an unexpected row: %+v", fetched)
	}
	t.Log("PASS get round-trips the written value")

	// TOGGLE — an update through the same channel flips `done`.
	fetched.Done = true
	toggled, err := json.Marshal(fetched)
	if err != nil {
		t.Fatalf("marshal toggle: %v", err)
	}
	if _, err := tbl.Put(key, toggled); err != nil {
		t.Fatalf("Put (toggle): %v", err)
	}
	got, found, err = tbl.Get(key)
	if err != nil || !found {
		t.Fatalf("Get after toggle: found=%v err=%v", found, err)
	}
	var afterToggle todo
	if err := json.Unmarshal(got, &afterToggle); err != nil {
		t.Fatalf("unmarshal toggled (%s): %v", got, err)
	}
	if !afterToggle.Done {
		t.Fatalf("toggle did not persist: %+v", afterToggle)
	}
	t.Log("PASS toggle persists done=true")

	// LIST — the collection carries the row.
	all, err := tbl.GetAll("limit=50")
	if err != nil {
		t.Fatalf("GetAll: %v", err)
	}
	var rows []todo
	if err := json.Unmarshal(all, &rows); err != nil {
		t.Fatalf("unmarshal collection (%s): %v", all, err)
	}
	if !containsID(rows, key) {
		t.Fatalf("list did not include the written row (%d rows)", len(rows))
	}
	t.Log("PASS list returns a JSON collection including the written row")

	// A missing record is an absent read, not an error (a record-miss 404 maps
	// to not-found, like the back-door store).
	if _, found, err = tbl.Get("nope-" + key); err != nil {
		t.Fatalf("Get (absent): %v", err)
	} else if found {
		t.Fatal("an absent key reported found")
	}
	t.Log("PASS absent key reports not-found over the front door")

	// DELETE — the row is gone.
	if err := tbl.Delete(key); err != nil {
		t.Fatalf("Delete: %v", err)
	}
	if _, found, err = tbl.Get(key); err != nil {
		t.Fatalf("Get after delete: %v", err)
	} else if found {
		t.Fatal("delete did not remove the row")
	}
	t.Log("PASS delete removes the row")

	// A missing APP is a typed route miss, never an absent row (YTC-1890): the
	// status and problem type are fields of *Error, not text to parse.
	_, _, err = fd.Table("no-such-app-"+key, table).Get("1")
	var sdkErr *Error
	if !errors.As(err, &sdkErr) || sdkErr.Kind != KindRemote {
		t.Fatalf("get on a missing app: want a remote *Error, got %v", err)
	}
	if sdkErr.Status != 404 || sdkErr.ProblemType != RouteNotFoundType || !sdkErr.IsRouteNotFound() {
		t.Fatalf("a missing app: want status 404 + %s, got %d %q", RouteNotFoundType, sdkErr.Status, sdkErr.ProblemType)
	}
	t.Log("PASS a missing app is a typed 404 route miss")

	// An anonymous read of the protected table is a typed 401.
	protected := envOr("YETI_FD_PROTECTED_TABLE", "api/Secret")
	_, _, err = fd.Table(app, protected).Get("any")
	if !errors.As(err, &sdkErr) || sdkErr.Kind != KindRemote || sdkErr.Status != 401 || sdkErr.ProblemType == "" {
		t.Fatalf("anonymous protected read: want a typed 401, got %v (%+v)", err, sdkErr)
	}
	t.Log("PASS an anonymous protected read is a typed 401")
}

// A malformed CA PEM fails deterministically in TlsConfig::pinned_roots (before
// any dial), so this proves the ClientError → Go *Error mapping without
// depending on a network timeout.
func TestFrontDoorBadCaPemSurfacesAsError(t *testing.T) {
	url := os.Getenv("YETI_FD_URL")
	if url == "" {
		t.Skip("YETI_FD_URL unset — run sdk-cells cell for the live-node fixture")
	}
	_, err := ConfigureFrontDoorPinned(url, envOr("YETI_FD_HOST", "localhost"), "not a valid pem", "", "")
	if err == nil {
		t.Fatal("a bad CA PEM was accepted")
	}
	var sdkErr *Error
	if !asSdkError(err, &sdkErr) {
		t.Fatalf("expected a *Error, got %T", err)
	}
	t.Logf("PASS a bad CA PEM surfaces as a typed error across the C ABI: %v", err)
}

func envOr(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}

func containsID(rows []todo, id string) bool {
	for _, r := range rows {
		if r.ID == id {
			return true
		}
	}
	return false
}

func asSdkError(err error, out **Error) bool {
	e, ok := err.(*Error)
	if ok {
		*out = e
	}
	return ok
}
