#pragma once
#include <vector>
#include <functional>
#include <stdexcept>

template <typename T, typename Compare = std::less<T>>
class BinaryHeap {
public:
    explicit BinaryHeap(Compare cmp = Compare{}) : cmp_(cmp) {}

    void push(const T &value) {
        data_.push_back(value);
        sift_up(data_.size() - 1);
    }

    void push(T &&value) {
        data_.push_back(std::move(value));
        sift_up(data_.size() - 1);
    }

    const T &top() const {
        if (empty()) throw std::underflow_error("heap is empty");
        return data_[0];
    }

    void pop() {
        if (empty()) throw std::underflow_error("heap is empty");
        std::swap(data_[0], data_.back());
        data_.pop_back();
        if (!empty()) sift_down(0);
    }

    bool empty() const noexcept { return data_.empty(); }
    std::size_t size() const noexcept { return data_.size(); }

private:
    std::vector<T> data_;
    Compare        cmp_;

    void sift_up(std::size_t idx) {
        while (idx > 0) {
            std::size_t parent = (idx - 1) / 2;
            if (cmp_(data_[idx], data_[parent])) {
                std::swap(data_[idx], data_[parent]);
                idx = parent;
            } else break;
        }
    }

    void sift_down(std::size_t idx) {
        std::size_t n = data_.size();
        while (true) {
            std::size_t smallest = idx;
            std::size_t left     = 2 * idx + 1;
            std::size_t right    = 2 * idx + 2;
            if (left  < n && cmp_(data_[left],     data_[smallest])) smallest = left;
            if (right < n && cmp_(data_[right],    data_[smallest])) smallest = right;
            if (smallest == idx) break;
            std::swap(data_[idx], data_[smallest]);
            idx = smallest;
        }
    }
};
