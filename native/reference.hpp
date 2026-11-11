#pragma once
#include <cmath>
#include <cstddef>
#include <stdexcept>
#include <vector>
namespace aegis {
struct FlowResult {
  double loss;
  std::vector<double> gradient;
};
inline FlowResult flow_mse(const std::vector<double> &prediction, const std::vector<double> &clean,
                           const std::vector<double> &noise) {
  if (prediction.empty() || clean.size() != prediction.size() || noise.size() != prediction.size())
    throw std::invalid_argument("flow shapes differ");
  FlowResult r{0., std::vector<double>(prediction.size())};
  for (std::size_t i = 0; i < prediction.size(); ++i) {
    if (!std::isfinite(prediction[i]) || !std::isfinite(clean[i]) || !std::isfinite(noise[i]))
      throw std::invalid_argument("nonfinite flow input");
    const double error = prediction[i] - (clean[i] - noise[i]);
    r.loss += error * error / prediction.size();
    r.gradient[i] = 2. * error / prediction.size();
  }
  if (!std::isfinite(r.loss))
    throw std::overflow_error("loss overflow");
  return r;
}
// Unified stream: text prefix is causal; media queries see all text and their own
// noisy media block. Separate media blocks cannot leak clean targets to text.
enum class Modality { text, video, audio };
struct Token {
  Modality modality;
  std::size_t position;
};
inline bool visible(Token query, Token key) {
  if (query.modality == Modality::text)
    return key.modality == Modality::text && key.position <= query.position;
  return key.modality == Modality::text || key.modality == query.modality;
}
} // namespace aegis
