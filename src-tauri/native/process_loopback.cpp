#include <audioclient.h>
#include <audioclientactivationparams.h>
#include <mmdeviceapi.h>
#include <windows.h>
#include <wrl.h>
#include <wrl/implements.h>

#include <new>

using Microsoft::WRL::ClassicCom;
using Microsoft::WRL::ComPtr;
using Microsoft::WRL::FtmBase;
using Microsoft::WRL::Make;
using Microsoft::WRL::RuntimeClass;
using Microsoft::WRL::RuntimeClassFlags;

namespace {

class ActivationHandler final
    : public RuntimeClass<RuntimeClassFlags<ClassicCom>, FtmBase,
                          IActivateAudioInterfaceCompletionHandler> {
public:
  ActivationHandler() : completed_(CreateEventW(nullptr, FALSE, FALSE, nullptr)) {}

  ~ActivationHandler() override {
    if (completed_ != nullptr) {
      CloseHandle(completed_);
    }
  }

  bool ready() const { return completed_ != nullptr; }
  HANDLE completed_event() const { return completed_; }
  HRESULT activation_result() const { return activation_result_; }

  HRESULT copy_client(IAudioClient **client) const {
    return audio_client_.CopyTo(client);
  }

  STDMETHODIMP ActivateCompleted(
      IActivateAudioInterfaceAsyncOperation *operation) override {
    HRESULT activation_status = E_UNEXPECTED;
    ComPtr<IUnknown> activated_interface;
    HRESULT result = operation->GetActivateResult(
        &activation_status, activated_interface.GetAddressOf());
    if (SUCCEEDED(result)) {
      result = activation_status;
    }
    if (SUCCEEDED(result)) {
      result = activated_interface.As(&audio_client_);
    }

    activation_result_ = result;
    SetEvent(completed_);
    return S_OK;
  }

private:
  HANDLE completed_ = nullptr;
  HRESULT activation_result_ = E_UNEXPECTED;
  ComPtr<IAudioClient> audio_client_;
};

struct ActivationContext {
  ComPtr<ActivationHandler> handler;
  ComPtr<IActivateAudioInterfaceAsyncOperation> operation;
};

} // namespace

extern "C" HRESULT ort_activate_process_loopback(
    DWORD excluded_process_id, DWORD timeout_ms, IAudioClient **audio_client,
    void **activation_context) {
  if (audio_client == nullptr || activation_context == nullptr) {
    return E_POINTER;
  }
  *audio_client = nullptr;
  *activation_context = nullptr;

  auto handler = Make<ActivationHandler>();
  if (!handler || !handler->ready()) {
    return E_OUTOFMEMORY;
  }

  AUDIOCLIENT_ACTIVATION_PARAMS audio_params{};
  audio_params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
  audio_params.ProcessLoopbackParams.TargetProcessId = excluded_process_id;
  audio_params.ProcessLoopbackParams.ProcessLoopbackMode =
      PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE;

  PROPVARIANT activation_params{};
  activation_params.vt = VT_BLOB;
  activation_params.blob.cbSize = sizeof(audio_params);
  activation_params.blob.pBlobData =
      reinterpret_cast<BYTE *>(&audio_params);

  ComPtr<IActivateAudioInterfaceAsyncOperation> operation;
  HRESULT result = ActivateAudioInterfaceAsync(
      VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, __uuidof(IAudioClient),
      &activation_params, handler.Get(), operation.GetAddressOf());
  if (FAILED(result)) {
    return result;
  }

  const DWORD wait_result =
      WaitForSingleObject(handler->completed_event(), timeout_ms);
  if (wait_result == WAIT_TIMEOUT) {
    return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
  }
  if (wait_result != WAIT_OBJECT_0) {
    return HRESULT_FROM_WIN32(GetLastError());
  }

  result = handler->activation_result();
  if (FAILED(result)) {
    return result;
  }

  auto *context = new (std::nothrow) ActivationContext{handler, operation};
  if (context == nullptr) {
    return E_OUTOFMEMORY;
  }

  result = handler->copy_client(audio_client);
  if (FAILED(result)) {
    delete context;
    return result;
  }

  *activation_context = context;
  return S_OK;
}

extern "C" void ort_release_process_loopback(void *activation_context) {
  delete static_cast<ActivationContext *>(activation_context);
}
