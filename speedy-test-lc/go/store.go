// Package store provides a thread-safe in-memory key-value store
// with optional TTL-based expiration.
package store

import (
	"sync"
	"time"
)

type entry struct {
	value     any
	expiresAt time.Time // zero means no expiry
}

func (e entry) expired() bool {
	return !e.expiresAt.IsZero() && time.Now().After(e.expiresAt)
}

// Store is a concurrent key-value store with optional TTL.
type Store struct {
	mu   sync.RWMutex
	data map[string]entry
}

// New creates an empty Store and starts a background janitor.
func New(cleanupInterval time.Duration) *Store {
	s := &Store{data: make(map[string]entry)}
	go s.janitor(cleanupInterval)
	return s
}

// Set stores value under key. A zero ttl means the key never expires.
func (s *Store) Set(key string, value any, ttl time.Duration) {
	var exp time.Time
	if ttl > 0 {
		exp = time.Now().Add(ttl)
	}
	s.mu.Lock()
	s.data[key] = entry{value: value, expiresAt: exp}
	s.mu.Unlock()
}

// Get retrieves the value for key. Returns (nil, false) when the key is
// missing or has expired.
func (s *Store) Get(key string) (any, bool) {
	s.mu.RLock()
	e, ok := s.data[key]
	s.mu.RUnlock()
	if !ok || e.expired() {
		return nil, false
	}
	return e.value, true
}

// Delete removes a key unconditionally.
func (s *Store) Delete(key string) {
	s.mu.Lock()
	delete(s.data, key)
	s.mu.Unlock()
}

// Len returns the number of live (non-expired) keys.
func (s *Store) Len() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	count := 0
	for _, e := range s.data {
		if !e.expired() {
			count++
		}
	}
	return count
}

func (s *Store) janitor(interval time.Duration) {
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for range ticker.C {
		s.mu.Lock()
		for k, e := range s.data {
			if e.expired() {
				delete(s.data, k)
			}
		}
		s.mu.Unlock()
	}
}
