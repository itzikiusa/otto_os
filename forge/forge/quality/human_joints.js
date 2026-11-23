ObjC.import('Foundation');
ObjC.import('Vision');
function run(args) {
  const request = $.VNDetectHumanBodyPoseRequest.alloc.init;
  request.usesCPUOnly = true;
  const handler = $.VNImageRequestHandler.alloc.initWithURLOptions($.NSURL.fileURLWithPath(args[0]), $({}));
  const error = Ref();
  if (!handler.performRequestsError($([request]), error)) throw Error(ObjC.unwrap(error[0].localizedDescription));
  const result = request.results;
  const people = [];
  for (let i=0;i<result.count;i++) {
    const observation = result.objectAtIndex(i);
    const names = ObjC.deepUnwrap(observation.availableJointNames);
    const points = {};
    for (const key of names) {
      const p = observation.recognizedPointForJointNameError($(key), error);
      if (p.confidence > 0.12) points[key] = [p.x,p.y,p.confidence];
    }
    people.push(points);
  }
  return JSON.stringify(people);
}
