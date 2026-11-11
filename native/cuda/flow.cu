// GPU primitive only: not a transformer or complete training backend.
#include <cstddef>
#include <cuda_runtime.h>
__global__ void flow_mse_kernel(const float *prediction, const float *clean, const float *noise,
                                float *error_squared, float *gradient, std::size_t n) {
  const std::size_t i = static_cast<std::size_t>(blockIdx.x) * blockDim.x + threadIdx.x;
  if (i < n) {
    const float e = prediction[i] - (clean[i] - noise[i]);
    error_squared[i] = e * e / static_cast<float>(n);
    gradient[i] = 2.f * e / static_cast<float>(n);
  }
}
extern "C" cudaError_t aegis_flow_mse(const float *prediction, const float *clean,
                                      const float *noise, float *per_element_loss, float *gradient,
                                      std::size_t n, cudaStream_t stream) {
  if (!prediction || !clean || !noise || !per_element_loss || !gradient || !n ||
      n > static_cast<std::size_t>(2147483647) * 256)
    return cudaErrorInvalidValue;
  flow_mse_kernel<<<static_cast<unsigned>((n + 255) / 256), 256, 0, stream>>>(
      prediction, clean, noise, per_element_loss, gradient, n);
  return cudaGetLastError(); // Caller must synchronize stream to observe runtime faults.
}
