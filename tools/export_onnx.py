#!/usr/bin/env python3
"""Export MegaDescriptor-T-224 (Swin-Tiny, 768-dim) to ONNX, then int8-quantize.

Dev-only, run once. Output goes to models/ (gitignored).

  python3 -m venv tools/.venv
  tools/.venv/bin/pip install torch --index-url https://download.pytorch.org/whl/cpu
  tools/.venv/bin/pip install timm onnx onnxruntime
  tools/.venv/bin/python tools/export_onnx.py
"""
import os
import sys
import torch
import timm

OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "models")
FP32 = os.path.join(OUT_DIR, "megadescriptor-t-224.onnx")
INT8 = os.path.join(OUT_DIR, "megadescriptor-t-224-int8.onnx")


def main() -> None:
    os.makedirs(OUT_DIR, exist_ok=True)

    print("== loading model ==")
    model = timm.create_model("hf-hub:BVRA/MegaDescriptor-T-224", num_classes=0, pretrained=True)
    model.eval()

    dummy = torch.randn(1, 3, 224, 224)
    with torch.no_grad():
        out = model(dummy)
    print("embedding shape:", tuple(out.shape))  # expect (1, 768)

    print("== exporting fp32 ONNX ==")
    torch.onnx.export(
        model,
        dummy,
        FP32,
        input_names=["input"],
        output_names=["embedding"],
        opset_version=17,
        do_constant_folding=True,
        dynamo=False,
    )
    print("wrote", FP32, os.path.getsize(FP32) // (1024 * 1024), "MB")

    print("== int8 dynamic quantization ==")
    from onnxruntime.quantization import quantize_dynamic, QuantType

    quantize_dynamic(FP32, INT8, weight_type=QuantType.QInt8)
    print("wrote", INT8, os.path.getsize(INT8) // (1024 * 1024), "MB")

    print("== verifying int8 loads & runs ==")
    import numpy as np
    import onnxruntime as ort

    so = ort.SessionOptions()
    so.intra_op_num_threads = 1
    so.enable_cpu_mem_arena = False
    sess = ort.InferenceSession(INT8, sess_options=so, providers=["CPUExecutionProvider"])
    y = sess.run(None, {"input": np.random.rand(1, 3, 224, 224).astype(np.float32)})
    print("int8 output shape:", y[0].shape)
    print("DONE")


if __name__ == "__main__":
    sys.exit(main())
