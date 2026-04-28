import { invoke } from "@tauri-apps/api/core";

export interface SignatureParam {
  label: string;
  documentation: string | null;
}

export interface FunctionSignature {
  schema: string;
  name: string;
  label: string;
  parameters: SignatureParam[];
  result: string;
  kind: string;
}

export async function signatureHelp(
  profileId: string,
  name: string,
): Promise<FunctionSignature[]> {
  return invoke<FunctionSignature[]>("signature_help", { profileId, name });
}
