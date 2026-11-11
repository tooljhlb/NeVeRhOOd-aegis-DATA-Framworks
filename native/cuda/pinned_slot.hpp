#pragma once
#include <cstddef>
#include <cuda_runtime.h>
#include <stdexcept>
namespace aegis {
// One reusable staging slot. Queue ownership determines lifetime. Destruction
// waits for its stream before freeing pinned host/device buffers.
class PinnedSlot {
  void *host_ = nullptr;
  void *device_ = nullptr;
  cudaStream_t stream_ = nullptr;
  std::size_t bytes_;
  static void check(cudaError_t e) {
    if (e != cudaSuccess)
      throw std::runtime_error(cudaGetErrorString(e));
  }

public:
  explicit PinnedSlot(std::size_t bytes) : bytes_(bytes) {
    if (!bytes)
      throw std::invalid_argument("zero slot size");
    try {
      check(cudaStreamCreateWithFlags(&stream_, cudaStreamNonBlocking));
      check(cudaHostAlloc(&host_, bytes, cudaHostAllocDefault));
      check(cudaMalloc(&device_, bytes));
    } catch (...) {
      if (device_)
        cudaFree(device_);
      if (host_)
        cudaFreeHost(host_);
      if (stream_)
        cudaStreamDestroy(stream_);
      throw;
    }
  }
  ~PinnedSlot() {
    if (stream_)
      cudaStreamSynchronize(stream_);
    if (device_)
      cudaFree(device_);
    if (host_)
      cudaFreeHost(host_);
    if (stream_)
      cudaStreamDestroy(stream_);
  }
  PinnedSlot(const PinnedSlot &) = delete;
  PinnedSlot &operator=(const PinnedSlot &) = delete;
  void *host() { return host_; }
  void *device() { return device_; }
  cudaStream_t stream() { return stream_; }
  void upload() { check(cudaMemcpyAsync(device_, host_, bytes_, cudaMemcpyHostToDevice, stream_)); }
  void wait() { check(cudaStreamSynchronize(stream_)); }
};
} // namespace aegis
