package store

import (
	"testing"
	"time"
)

func TestSetGet(t *testing.T) {
	s := New(time.Minute)
	s.Set("name", "alice", 0)
	v, ok := s.Get("name")
	if !ok || v != "alice" {
		t.Fatalf("expected 'alice', got %v, %v", v, ok)
	}
}

func TestMissingKey(t *testing.T) {
	s := New(time.Minute)
	_, ok := s.Get("ghost")
	if ok {
		t.Fatal("expected false for missing key")
	}
}

func TestTTLExpiry(t *testing.T) {
	s := New(time.Millisecond * 10)
	s.Set("temp", 42, time.Millisecond*20)
	time.Sleep(time.Millisecond * 50)
	_, ok := s.Get("temp")
	if ok {
		t.Fatal("expected key to be expired")
	}
}

func TestDelete(t *testing.T) {
	s := New(time.Minute)
	s.Set("x", 1, 0)
	s.Delete("x")
	_, ok := s.Get("x")
	if ok {
		t.Fatal("expected deleted key to be missing")
	}
}
