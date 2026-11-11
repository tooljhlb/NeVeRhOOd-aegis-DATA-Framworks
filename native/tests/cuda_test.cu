#include "../cuda/pinned_slot.hpp"
#include <cmath>
#include <iostream>
#include <vector>
extern "C" cudaError_t aegis_flow_mse(const float *, const float *, const float *, float *, float *,
                                      std::size_t, cudaStream_t);
int main() {
  constexpr std::size_t n = 3;
  aegis::PinnedSlot prediction(n * sizeof(float)), clean(n * sizeof(float)),
      noise(n * sizeof(float)), loss(n * sizeof(float)), gradient(n * sizeof(float));
  auto p = static_cast<float *>(prediction.host());
  auto x = static_cast<float *>(clean.host());
  auto z = static_cast<float *>(noise.host());
  for (std::size_t i = 0; i < n; ++i) {
    p[i] = 0.2f * i;
    x[i] = float(i + 1);
    z[i] = -0.1f * i;
  }
  prediction.upload();
  clean.upload();
  noise.upload();
  prediction.wait();
  clean.wait();
  noise.wait();
  auto code = aegis_flow_mse(
      static_cast<float *>(prediction.device()), static_cast<float *>(clean.device()),
      static_cast<float *>(noise.device()), static_cast<float *>(loss.device()),
      static_cast<float *>(gradient.device()), n, prediction.stream());
  if (code != cudaSuccess)
    return 1;
  prediction.wait();
  std::vector<float> g(n);
  if (cudaMemcpy(g.data(), gradient.device(), n * sizeof(float), cudaMemcpyDeviceToHost) !=
      cudaSuccess)
    return 2;
  for (std::size_t i = 0; i < n; ++i)
    if (std::abs(g[i] - 2.f * (p[i] - (x[i] - z[i])) / n) > 1e-5f)
      return 3;
  std::cout << "PASS CUDA flow gradient parity\n";
}
