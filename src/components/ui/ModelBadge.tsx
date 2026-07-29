import { Badge } from './Badge';
import { MODEL_VARIANT, modelKey, modelLabel } from '../../report/modelDisplay';

interface Props {
  /** Raw model id from a transcript or provider config, e.g.
   *  `claude-opus-5[1m]`, `glm-5.2`. */
  model: string | null | undefined;
  className?: string;
}

/**
 * The one way a model is shown anywhere in the app: a pill tinted by family
 * (opus/sonnet/haiku keep their tonal colors, everything else is neutral)
 * carrying the cleaned-up label.
 *
 * Renders nothing when the model is unknown. A transcript with no assistant
 * `model` field is not "the unknown model" — printing that word puts a
 * same-weight chip in the column where every other row carries real
 * information, which is precisely the noise the badge exists to cut.
 */
export function ModelBadge({ model, className = '' }: Props) {
  if (!model) return null;
  return (
    <Badge variant={MODEL_VARIANT[modelKey(model)] ?? 'default'} className={className}>
      {modelLabel(model)}
    </Badge>
  );
}
