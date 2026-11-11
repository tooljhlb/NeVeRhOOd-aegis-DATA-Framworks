#include "../reference.hpp"
#include <iostream>
#include <limits>
#define CHECK(expr)                                                                                \
  do {                                                                                             \
    if (!(expr))                                                                                   \
      throw std::runtime_error(#expr);                                                             \
  } while (0)
int main() {
  using namespace aegis;
  std::vector<double> p{0.2, -0.3, 0.4}, x{1., 2., 3.}, z{-1., 0., 2.};
  auto r = flow_mse(p, x, z);
  for (std::size_t i = 0; i < p.size(); ++i) {
    auto a = p, b = p;
    a[i] += 1e-6;
    b[i] -= 1e-6;
    double numerical = (flow_mse(a, x, z).loss - flow_mse(b, x, z).loss) / 2e-6;
    CHECK(std::abs(numerical - r.gradient[i]) < 1e-8);
  }
  CHECK(visible({Modality::text, 2}, {Modality::text, 1}));
  CHECK(!visible({Modality::text, 1}, {Modality::text, 2}));
  CHECK(!visible({Modality::text, 2}, {Modality::video, 0}));
  CHECK(visible({Modality::video, 0}, {Modality::text, 10}));
  CHECK(!visible({Modality::video, 0}, {Modality::audio, 0}));
  bool rejected = false;
  try {
    flow_mse({}, x, z);
  } catch (const std::invalid_argument &) {
    rejected = true;
  }
  CHECK(rejected);
  rejected = false;
  p[0] = std::numeric_limits<double>::quiet_NaN();
  try {
    flow_mse(p, x, z);
  } catch (const std::invalid_argument &) {
    rejected = true;
  }
  CHECK(rejected);
  std::cout
      << "PASS: finite-difference flow gradients, attention visibility, invalid-input checks\n";
}